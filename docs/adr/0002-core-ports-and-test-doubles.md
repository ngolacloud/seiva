# ADR 0002 — Core ports and in-memory test doubles

- Status: Accepted (the `UnitOfWork` shape amended by ADR 0004)
- Date: 2026-11-03
- Deciders: NgolaCloud
- Follows: ADR 0001

> **Amendment (ADR 0004).** This ADR first described `UnitOfWork` as
> handle-based (`begin()` → `commit()`/`rollback()` + `on_commit`). The DORM
> gateway revealed that DORM's executor is closure-scoped (`db::atomic`), which a
> handle-based commit cannot drive, so `UnitOfWork` is now **scope-based**:
> `transaction(|| async { … })` commits on `Ok` and rolls back on `Err`, and the
> "publish after commit" need is met by acting on the returned `Ok` (no
> `on_commit` hook). The other three ports are unchanged.

## Context

The application layer of every module must be writable, and testable, without a
server, a broker or a database, and without depending on the framework crate or
on DORM. That is only possible if the capabilities a use case needs are named as
small, stable traits (ports) that the use case codes against, with concrete
implementations supplied at composition time and fakes supplied under test.

This ADR fixes the core port set, the error model at the boundary, how
dependencies are resolved, and where the in-memory doubles live. It is the
ground every later phase (data layer, runtime, scaffolding) builds on, so the
shapes here are chosen to be stable: `seiva-ports` is SemVer-strict by policy.

## Decision

### 1. The core ports

`seiva-ports` holds exactly four traits, the ones every module relies on
regardless of its data layer:

- **`Clock`** — reads the current time (`now() -> SystemTime`). Injected so use
  cases are deterministic: a fake returns a fixed instant.
- **`IdGenerator`** — mints identifiers (`new_id() -> Id`). Injected so a test
  double hands out a predictable sequence. `Id` is a dependency-free
  `[u8; 16]`; the framework and adapters render it as UUID/ULID.
- **`EventPublisher`** — publishes a domain event (`publish(topic, payload)`).
  The payload is already serialized by the caller; whether the publish is
  transactional (outbox) or best-effort is a property of the implementation and
  of `seiva.toml`, not of the trait.
- **`UnitOfWork`** / **`UnitOfWorkFactory`** — the transaction boundary. The
  application layer decides where a transaction begins and ends, one per use
  case, through `begin()` → `commit()`/`rollback()`. `on_commit` registers a
  callback that runs after a successful commit only (e.g. to wake the outbox
  relay), and is discarded on rollback.

Data-layer ports (`Connection` health/shutdown, `OutboxStore`, `JobStore`) are
deferred to the data-layer phase and added then; keeping them out now keeps this
boundary minimal until there is a real implementation to shape them against.

### 2. Error model

Ports return `PortResult<T> = Result<T, BoxError>`, where `BoxError` is a boxed
`std::error::Error + Send + Sync`. The boundary carries no error library, so it
imposes none on downstream code; adapters map their concrete errors (a DB error,
a broker error) into `BoxError` at the edge, and the application layer sees one
uniform failure type. Domain errors are a separate concern owned by the
`domain`/`application` crates and never cross this boundary as strings.

### 3. Dependencies resolved at compile time

A use case declares the ports it needs in its signature (generics or
`dyn Port`). Composition wires concrete implementations; there is no runtime
reflection and no service locator. A missing provider is a build error, not a
runtime panic, and a cycle between modules is a build error too. Under test, a
double is substituted directly (the brief's `app.override::<dyn Port>(fake)`),
which works precisely because the use case depends on the trait, never on the
implementation.

### 4. In-memory doubles live in `seiva-testing`

For each port, Seiva provides a hand-written reference double in a dedicated
`seiva-testing` crate that depends only on `seiva-ports` (plus `async-trait`).
Modules pull it in as a `dev-dependency`, so the doubles never enter a release
build and the application layer's runtime dependency stays `seiva-ports` alone.

The doubles are **not** placed in `seiva-ports`: that crate is trait-only, with
no logic, and must stay so. When the scaffolding phase generates per-module
doubles (the brief's "o Seiva gera um duplo em memória"), these reference doubles
are the shape it emits and the fallback a module can use directly.

The founding doubles:

| Port | Double | Behaviour |
| --- | --- | --- |
| `Clock` | `FixedClock` | Fixed at a chosen instant; `advance(Duration)` for time-based assertions |
| `IdGenerator` | `SequenceIdGenerator` | Deterministic ids from a counter (big-endian into `Id`) |
| `EventPublisher` | `RecordingEventPublisher` | Records every `(topic, payload)` for assertions |
| `UnitOfWorkFactory` | `InMemoryUnitOfWorkFactory` | Records each transaction's outcome; the `UnitOfWork` runs `on_commit` hooks on commit and discards them on rollback |

### 5. `seiva-ports` stability policy

`seiva-ports` is SemVer-strict: a breaking change to a port is a major version.
It stays trait-only — no runtime, no macros — with `async-trait` as its single
dependency (needed for object-safe async methods across the `dyn Port`
boundary). This is what lets every module and every generated double code
against a boundary that does not churn.

## Consequences

- An application-layer unit test needs no `tokio` beyond an executor, no DB and
  no broker: construct the handler with the four doubles and assert on recorded
  events and transaction outcomes.
- The `on_commit`-on-commit-only semantics are observable and tested, so the
  later outbox-relay wake-up is built on a proven contract.
- Adding a data-layer port later is additive (new trait), not a breaking change
  to the four fixed here.

## Alternatives considered

- **Doubles behind a `testing` feature of `seiva-ports`.** Rejected: it would
  put logic (interior mutability, recording) into the trait-only crate, breaking
  its "traits only" invariant even though the deps would stay std-only.
- **A runtime service locator / DI container.** Rejected: it moves "missing
  dependency" from compile time to runtime, which a high-criticality framework
  must not do. DI is resolved by the type system.
- **A concrete error enum on the ports.** Rejected: it would force an error
  taxonomy onto the boundary and leak implementation detail (which backend
  failed) into the application layer. `BoxError` keeps the boundary neutral.
