# ADR 0011 — Durable business idempotency as a port, reusing DORM

- Status: Accepted
- Date: 2026-11-05
- Deciders: NgolaCloud
- Follows: ADR 0002 (ports), ADR 0004 (DORM gateway)

## Context

A command with effects — a `POST` that creates a record, a job, an admin action
— can be retried after a timeout or a lost response. Without an idempotency
guarantee the retry runs the effect twice. The brief (§7) requires a **reusable,
durable, distributed** idempotency guarantee, and is specific:

- The base framework must **not** depend on a particular database; reuse existing
  DORM/outbox primitives through a port.
- Concurrency must be arbitrated by a durable uniqueness/CAS, **not** an in-memory
  lock (which cannot coordinate across replicas).
- The reservation, the domain mutation and the outbox write should commit in
  **one** transaction, so a committed key always has its effect and vice versa.
- Same key + different request → `409` without executing; completed → replay the
  stored result without re-executing; in-flight → bounded wait or conflict.
- An in-memory `HashMap` test does **not** demonstrate the distributed guarantee.

DORM already ships the primitive (`dorm_core::idempotency`): `begin(key,
request_hash, ttl)` claims a key with an atomic `INSERT … ON CONFLICT DO NOTHING
RETURNING` and returns `Fresh`/`Replay(json)`/`InProgress`; a reused key with a
different hash is `IdempotencyMismatch`; `finish` stores the response; the rows
participate in the ambient DORM session/transaction.

## Decision

**Define an `IdempotencyStore` port in `seiva-ports` and implement it in the DORM
gateway by delegating to `dorm_core::idempotency`. The base framework codes
against the port; the durable, cross-replica guarantee lives in the adapter.**

- `seiva_ports::IdempotencyStore` (dependency-free, byte-oriented like
  `EventPublisher`): `begin(scoped_key, fingerprint, ttl) -> IdempotencyOutcome`
  (`Fresh` | `Replay(bytes)` | `InProgress` | `Mismatch`) and `finish(scoped_key,
  response_bytes)`. The key is **scoped by the caller** (tenant + identity +
  operation + version folded in) before it reaches the store, so tenants' keys
  never collide; the `fingerprint` pins the key to one request's
  semantically-relevant bytes. `Mismatch` is an outcome, not an error, so the
  caller answers `409` without executing.
- `seiva_data_dorm::DormIdempotencyStore` maps the port onto
  `dorm_core::idempotency::{begin, finish}` and re-exports its DDL as
  `idempotency_migrations()`. Called **inside** `DormUnitOfWork::transaction`, the
  reservation commits in the same transaction as the domain mutation and the
  outbox write (`DormOutboxPublisher`) — the atomicity the brief requires. The
  opaque response bytes are stored losslessly in the JSONB column as a byte array.
- `seiva_testing::InMemoryIdempotencyStore` is a single-process double for
  **logic** tests only; it is documented as NOT a distributed proof.

### Proof (the distributed guarantee, against a real database)

`seiva-data-dorm`'s `live_idempotency_one_effect_under_concurrency` (gated on
`SEIVA_DORM_TEST_DSN`) drives **20 concurrent connections** — the replica model —
at one key, each running `begin → effect → finish` in a single transaction via
the gateway. Because the claim is a database CAS and the row lock serialises
duplicates, exactly one connection is `Fresh` and the side-effect table ends with
exactly **one** row; a different fingerprint is `Mismatch` (no effect); the
completed key `Replay`s the exact stored bytes; a different key is independent. It
passes against live PostgreSQL 16 — not an in-memory model.

## Consequences

**Positive.** One reusable, durable idempotency guarantee, arbitrated by the
database (works across replicas); atomic with the mutation and the outbox; the
base stays DB-agnostic behind the port; `Mismatch`/`Replay`/`InProgress` are
first-class; proven against a real database.

**Negative / cost.** The live proof needs a PostgreSQL (CI token-gated, like the
other DORM jobs). The response is stored as a JSONB byte array (simple, lossless;
slightly larger than base64 — revisit if large bodies become common).

### HTTP integration (delivered)

The transport's half lives in `seiva_http::idempotency`: an
`OptionalIdempotencyKey` extractor (validates the header — bounded length,
visible ASCII; absent is `None`, malformed is `400`), `scoped_key` (folds tenant
+ subject + method + path into the key so tenants/identities never collide or
replay one another), `fingerprint` (canonical SHA-256 of method + path + query +
the body as sorted-key JSON), and `mismatch_problem`/`in_progress_problem`
(`409`). A **generic outer middleware is deliberately avoided**: it runs above
any transaction and so could not keep the reservation atomic with the mutation +
outbox. Instead the application drives `IdempotencyStore::begin` inside its
`UnitOfWork` transaction; an end-to-end test
(`seiva-http/tests/idempotency_http.rs`) shows the full pattern (one effect on
`Fresh`, replay on retry, `409` on mismatch, `400` on a malformed key).

### Consumer inbox & TTL purge (delivered)

- **Inbox dedup.** `seiva_ports::inbox_key(consumer, event_id)` composes the store
  key for consumer-side deduplication; a consumer claims it through the same
  `IdempotencyStore::begin` inside the consume transaction (effect + mark
  together, ack after commit), so a broker redelivery is consumed once. Proved
  live (`live_inbox_dedup_and_purge`: the same event delivered twice → one
  effect) and offline (`seiva-testing`).
- **TTL purge.** `IdempotencyStore::purge_expired` (DORM → `purge_expired` SQL)
  plus `seiva_server::run_idempotency_purge`, a scheduled worker over
  `run_worker` that purges inside a `UnitOfWork` transaction. Proved live (a
  `ttl=0` key is purged and then reclaimable `Fresh`) and the worker glue is
  tested with the in-memory doubles.

## Alternatives considered

- **A new idempotency table owned by Seiva.** Rejected: DORM already has the
  primitive; the brief says reuse it through a port, and duplicating the schema
  would fork the guarantee.
- **Redis/in-memory coordination.** Rejected as the authority: it cannot be
  atomic with the PostgreSQL mutation + outbox, which is exactly the guarantee
  required. (It may later serve as a cache/coordination hint, not the authority.)
- **Reservation in a separate transaction from the effect.** Rejected for the
  default: it admits a committed key with no effect (a crash in between) needing
  reconciliation. One transaction makes "key committed ⇒ effect committed" hold
  by construction; `InProgress` remains available for designs that split them.

## Addendum (2026-11-05) — wired on a write hot path via the `&Ctx` seam

The port is now exercised end to end on a real HTTP write (the `Catalog.create`
of `apps/admin-data-e2e`), proving the transport half joins the durable half
without a bespoke middleware and without a new controller-macro parameter:

- `seiva_http::Ctx` (the server-side seam that already carried the `Principal`
  for tenant-scoping, ADR 0006/0009) now also carries the request's **method +
  path** and the optional, validated **`Idempotency-Key`**, populated in its
  `FromRequestParts`. A present-but-malformed key is treated as absent (a plain,
  non-deduplicated create) and never stored — the validation still guards the
  storage key. A handler wanting a hard `400` uses the `OptionalIdempotencyKey`
  extractor directly.
- The handler folds `scoped_key(principal, method, path, key)` and a canonical
  `fingerprint(method, path, query, body)` (the body via `serde_json::to_value`
  of the typed command — canonical, key-sorted) and calls `begin`/`finish`
  **inside** the `DormUnitOfWork` transaction: on `Fresh` it inserts and stores
  the response bytes; on `Replay` it returns the stored response; on
  `InProgress`/`Mismatch` it returns `409` without executing. The `409` is
  carried by a status-bearing facade error, so it is not remapped to `422`.

Proved live: `live_catalog_create_is_idempotent` (retry replays the identical
stored response; a key reused with different content is `409`; exactly one row),
and Proof C of `benchmarks/production/delonix/` — 40 concurrent identical POSTs
with one key, through the load balancer onto two replicas, insert exactly once.
