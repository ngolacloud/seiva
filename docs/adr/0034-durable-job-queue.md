# ADR 0034 — `seiva-jobs`: a durable job queue with leases on top of the `seiva-worker` loop

- Status: Proposed
- Date: 2026-11-23
- Deciders: NgolaCloud
- Follows: ADR 0011 (durable idempotency), ADR 0012 (cloud-native health and limits), ADR 0021 (in-process event bus and choreography), ADR 0028 (worker concurrency safety)

## Context

Seiva has a worker **loop** but no worker **queue**. `seiva-worker::run_worker`
is exactly the half the architecture assigns to the framework: a generic loop that
repeats a one-pass function with an interval, fast-drain, graceful shutdown and
metrics — "the one-pass functions belong to the data layer … the loop is the
framework's". But the *passes* that exist are all bespoke: the outbox relay's
`forward`, RAG's `run_once`. There is **no general-purpose durable queue** a
product can enqueue arbitrary work onto and trust it to run exactly-enough-once,
survive a crash, retry, and dead-letter.

Every platform needs that. delonix-meet enqueues transcription, ffmpeg
transcoding, export generation and outbound mail — long, failable, retry-worthy
work that must not run on the request path and must survive a restart. Today each
is a hand-rolled table-poll loop. ADR 0028 already decided the **safety** model
for competitive workers (`FOR UPDATE SKIP LOCKED`, a lease, atomic
claim-effect-mark); what is missing is the generic **queue** that packages that
model as a reusable primitive: `enqueue`, `claim`, `ack`/`retry`/`fail`,
dead-letter, scheduling. This is the horizontal mechanism; the jobs themselves are
the product's policy.

The forces:

- **The loop exists; the queue is new.** `run_worker` already carries timing,
  drain and shutdown. `seiva-jobs` provides the *pass* it drives: claim a batch of
  due jobs under a lease, hand each to a handler, record the outcome. It must not
  re-derive the loop.
- **A lease, not a mark.** A claimed job holds a **visibility timeout**: if the
  worker dies mid-handle, the lease expires and the job returns to the queue and is
  re-claimed — at-least-once, never stuck (the recovery property ADR 0028 proved).
- **Safe at N replicas by construction.** Competitive consumers must share the
  queue via `SKIP LOCKED` with no leader and no single-reader bottleneck (ADR
  0028), and each effect must be idempotent through the inbox (ADR 0011), so
  scaling the worker out is the safe default, not a footgun.
- **A port, not a table.** The queue is defined as a `JobQueue` port with a
  DORM/`sqlx` backend as the default and a Redis backend as an option — the data
  layer stays pluggable (same line as ADR 0030). The core stays DORM-free.
- **Scheduling is part of the mechanism.** Delayed jobs (`run_at`) and recurring
  jobs (a cron expression) are generic; the product should not re-implement a
  scheduler to send a reminder in an hour or run a nightly export.
- **Graceful drain.** On shutdown (ADR 0012) a worker must stop claiming, finish or
  release its in-flight leases, and exit — no job is lost and none is left locked
  past the drain.

## Decision

Add a new workspace member **`seiva-jobs`** — the durable queue; the loop stays in
`seiva-worker`, the safety model stays ADR 0028's.

- **`JobQueue`** — the port:
  - `enqueue(job: NewJob) -> JobId` — persist a job with a `kind`, an opaque
    payload, a `Scope` (tenant stamped on the row), an optional `run_at`, a
    `max_attempts`, and an optional idempotency key (a duplicate enqueue under the
    same key is a no-op, ADR 0011).
  - `claim(kinds, limit, lease) -> Vec<LeasedJob>` — atomically select up to
    `limit` **due** jobs (`status = 'pending' AND run_at <= now`) with
    `FOR UPDATE SKIP LOCKED`, stamp `owner` + `lease_expires_at`, mark
    `in_flight`, commit (ADR 0028). A job whose lease expires returns to `pending`.
  - `ack(job)` — mark `done`; `retry(job, backoff)` — re-schedule with
    `next_attempt_at` and an incremented `attempt`; `fail(job)` / dead-letter — on
    exhausting `max_attempts` move to `dead` for inspection/replay.
- **The claim pass drives `run_worker`.** A `job_pass(queue, handlers)` is a
  one-pass function (claim → dispatch each to its `kind` handler inside one
  `UnitOfWork::transaction`, the atomic claim-effect-mark of ADR 0028 → ack/retry)
  returning how many it handled; `seiva-worker` loops it with drain-fast and
  graceful shutdown. No new loop is written.
- **Backends behind the port:**
  - **`DormJobQueue`** (default, feature-gated) — a `jobs` table, `SKIP LOCKED`
    claim, indexed on `(status, run_at)`; the authoritative durable store.
  - **`RedisJobQueue`** (optional) — for lower-latency, lower-durability work.
  - Poll uses the worker's drain-fast + interval; a `LISTEN/NOTIFY` wake is an
    optimisation over busy-polling where the backend supports it.
- **Scheduling.** `run_at` gives delayed jobs; a `schedule(cron, kind, payload)`
  registers a recurring job a scheduler pass materialises into due jobs — generic,
  so the product never writes a cron loop.
- **Idempotent handlers.** A job handler claims its inbox key (ADR 0011) inside the
  same transaction, so an at-least-once redelivery runs the effect once.

`tokio` enters `default-features = false` (`sync`, `time`); the DORM/Redis backends
are optional features, so the default build links no queue backend.

## Alternatives considered

- **Reach for an external broker (RabbitMQ/Kafka/SQS) as the primitive.** Rejected as
  the default: it adds an operational dependency for work that the database the product
  already runs can carry correctly via `SKIP LOCKED` + leases (ADR 0028). A broker can
  still be a `JobQueue` backend for a deployment that wants one — the port makes it a
  choice, not a requirement.
- **A second worker loop inside `seiva-jobs`.** Rejected: `run_worker` already carries
  interval, drain-fast, shutdown and metrics. Duplicating it would fork the loop
  semantics; the queue provides a *pass*, the worker provides the *loop*.
- **Leader election / pin the worker to one replica.** Rejected for the same reason as
  ADR 0028: it caps throughput at one worker and needs a coordinator the framework does
  not ship. Competitive `SKIP LOCKED` consumers scale out with no leader.
- **A fire-and-forget `tokio::spawn` for background work.** Rejected: a spawned task
  dies with the process, loses the job on a crash, and cannot retry or dead-letter. The
  whole point is durability across restarts.

## Consequences

- **Performance.** Batch `claim` with `SKIP LOCKED` and an index on `(status, run_at)`
  partitions work across replicas without contention; the worker's drain-fast path
  empties a backlog without idle waits; `LISTEN/NOTIFY` replaces busy-polling where
  available. The crate links no backend unless a feature is on — zero-cost when unused.
- **Security.** Each job carries its `Scope` (tenant stamped on the row) so a handler
  runs under the right tenant and cannot read across tenants; payloads are the product's
  and should be non-sensitive or encrypted at rest; the queue never logs payload
  contents.
- **Resilience.** A **lease with a visibility timeout** recovers a job whose worker died
  mid-handle (at-least-once, never stuck, ADR 0028); retry with bounded backoff and a
  **dead-letter** after `max_attempts` bound the blast radius of a poison job; idempotent
  handlers through the inbox (ADR 0011) make the at-least-once guarantee safe; graceful
  drain on shutdown (ADR 0012) finishes or releases in-flight leases so nothing is lost
  or left locked.
- delonix-meet enqueues transcription, ffmpeg, export and mail as **jobs** (its policy —
  the `kind` handlers) and deletes its hand-rolled poll loops; the generic queue,
  leasing, retry and scheduling become the framework's.
- It does **not** replace a stream-processing platform for high-throughput event pipelines:
  this is a durable task queue for a product's own asynchronous work, on the database it
  already runs — not a Kafka.
