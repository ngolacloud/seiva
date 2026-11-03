# ADR 0004 — The DORM data-layer gateway

- Status: Accepted
- Date: 2026-11-03
- Deciders: NgolaCloud
- Follows: ADR 0001, ADR 0002, ADR 0003

## Context

DORM is the default data layer, and the brief's Parte C requires that **all**
coupling to DORM live in one crate, so turning the data layer off is simply not
compiling that crate (the Gateway pattern). The rest of Seiva — the core, the
module `domain`/`application`/`facade`/`interface` crates — must know only
`seiva-ports`.

DORM has now merged the campaign this gateway depends on (DORM-C boundary, B1
strict sessions + cursor key by API + `connect_with`, B2 namespaces), at `main`
`f43d873`. The brief's rule holds until DORM publishes the library API: Seiva
depends on DORM **by a pinned git revision, never by local path**.

## Decision

### 1. One gateway crate, pinned by git revision

The `seiva-data-dorm` crate is the only place that names DORM. It depends on
`dorm-core` by `{ git = "…/dorm", rev = "f43d873…" }` and on `seiva-ports`. A
module's `infrastructure` crate uses it to build its DORM handle and to satisfy
the ports its `application` declared; nothing else imports DORM.

### 2. Not a workspace member yet

Building the gateway needs access to the private DORM repository. The Seiva CI
does not have that without a secret. So the gateway is **excluded** from the
workspace for now and built on its own (with `CARGO_NET_GIT_FETCH_WITH_CLI=1`),
which keeps the framework's CI green and DORM-free. Wiring it into the
framework's `dorm` feature — which makes the default build pull DORM and
requires a CI token (deploy key / PAT) — is a deliberate later step and the
owner's call.

### 3. The gateway enforces the DORM-B1 invariants on connect

`connect(DormConfig)` builds a module's `Database` with `connect_with` (DSN,
replicas, schema, pool by API — DORM-B1), and before returning it turns **strict
sessions on** (`set_strict_sessions(true)` — a statement with no session is an
error, never the process-global default) and **configures the cursor signing
key** (`cursor::set_cursor_key` — no forgeable default). A framework cannot
forget these: the gateway owns them.

### 4. Ports that map cleanly are implemented now

`Clock` (`SystemClock`) and `IdGenerator` (`UuidIdGenerator`, UUIDv4 → `Id`) are
implemented. These need no DORM, but they live here so a module's composition
has one data-layer provider to wire.

### 5. `UnitOfWork` and `EventPublisher` are deferred — a tx-model mismatch

The `seiva-ports` `UnitOfWork` is **handle-based**: `begin()` → `commit()` /
`rollback()`, with the handle outliving the call that opened it. DORM's
transaction API is **closure-scoped**: `db::atomic(|tx| async { … })` owns the
transaction for the duration of a closure and commits when it returns. The two
do not compose directly — a handle-based `commit()` later cannot drive a closure
that has already returned.

This is the first real finding of the boundary crossing, and it is resolved
next, one of two ways (decided in a follow-up ADR once prototyped):

- **a scope-based `UnitOfWork` port** — `run<F>(&self, f: F)` that the gateway
  maps straight onto `db::atomic`; the brief allows adjusting the port's shape
  ("ajusta os detalhes na ADR do núcleo"), and a scope-based unit of work is
  honest about where the transaction begins and ends; or
- **a handle-based DORM primitive** — DORM-B3's `on_commit`/explicit begin/commit
  on a pinned connection, which would let the current port shape stand.

`EventPublisher` follows the same resolution: the transactional outbox
(`dorm-outbox`) writes in the same transaction, so it is wired once the
transaction boundary is.

## Consequences

- The boundary crossing is proven: `seiva-data-dorm` compiles and tests against
  DORM pinned at `f43d873`, with `connect` building a handle and enforcing strict
  sessions + the cursor key (deadpool is lazy, so the test needs no database).
- The core stays DORM-free; the default `cargo build --workspace` pulls no DORM.
- The `UnitOfWork` port shape is now a known, scoped decision rather than an
  accidental one.

## Alternatives considered

- **Coupling the whole workspace to DORM now** (make the gateway a member and
  give CI a DORM token). Rejected for this step: it is the owner's call and would
  turn the green, self-contained CI into one that needs a secret.
- **Re-exporting DORM types through the gateway's public API.** Avoided beyond
  what is unavoidable (`Database`, `dorm_core::Error` on `connect`): the gateway
  is the DORM seam, but the ports keep DORM out of every other crate.
