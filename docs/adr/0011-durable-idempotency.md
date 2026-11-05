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

### Not in this ADR (next increments)

- An HTTP middleware that reads `Idempotency-Key`, composes the scoped key +
  fingerprint from the request, and short-circuits `Replay`/`Mismatch`/`InProgress`
  (buffering and replaying the response) — a consumer of the port.
- A consumer-side inbox/dedup (by event id + consumer) for the broker path.
- `purge_expired` wired to a scheduled worker for TTL cleanup.

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
