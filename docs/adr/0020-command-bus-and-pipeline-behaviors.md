# ADR 0020 — The command/query bus is wired into the pipeline with composable behaviors (opt-in)

- Status: Accepted
- Date: 2026-11-08
- Deciders: NgolaCloud
- Follows: ADR 0002 (ports), ADR 0003 (module layering), ADR 0006 (identity), ADR 0011 (durable idempotency), ADR 0019 (fail-closed authorization)

## Context

Seiva already ships a CQRS dispatch core. `seiva-cqrs` has typed `Command`/`Query`
messages, `CommandHandler`/`QueryHandler` use cases, and a `Mediator` whose
dispatch is **statically typed and zero-cost**: it is generic over the handler it
holds, so `send`/`query` resolve to a direct, monomorphised call with no boxing
and no `TypeId` lookup — a missing handler is a *type error*, not a runtime panic
(`crates/seiva-cqrs/src/lib.rs`). It even demonstrates the middleware seam with
`Instrument` (counts dispatches, the `#[policy]` hook) and `CacheQueries`
(memoises one read-side query, the `#[cached]` hook), composed by nesting:
`Mediator::new(Instrument::new(CacheQueries::new(handler)))`.

But the core is **not on the request path**. Two gaps:

- **The bus is disconnected from transport.** `#[seiva::controller]` generates a
  router whose handlers call the facade/service method *directly*
  (`self.0.place(cmd).await`, `examples/shop/orders/interface/src/lib.rs`); the
  `Mediator` appears only in that module's *tests*. No production request crosses
  it.
- **Cross-cutting concerns are scattered.** Authn and rate-limit are tower layers
  (`seiva-http`), authz is a per-route guard mounted by the macro
  (ADR 0019), idempotency is driven by the application *inside* its transaction
  (ADR 0011), validation is the `Valid<T>` extractor. Each is correct, but there
  is no single, typed place where "what must happen around a write" is expressed
  per use case — and the `#[policy]`/`#[cached]` seams on the facade are
  recognised by the macro yet produce **no behavior** (`seiva-macros/src/lib.rs`).

The forces:

- Give a use case **one typed pipeline** for its cross-cutting concerns, without
  giving up the compile-time, zero-cost dispatch Seiva is built on.
- Respect a hard structural fact: **idempotency and the outbox write must live
  *inside* the transaction** (ADR 0011 rejected an outer middleware precisely
  because it runs above any transaction and cannot stay atomic with the mutation).
  So the pipeline cannot be a flat stack of layers that all sit above the handler.
- Do not rewrite the HTTP path that was just hardened and turned green (PR #29).
  The change must be reversible and adoptable one use case at a time.
- Be transport-agnostic: an HTTP controller, a gRPC service and an event consumer
  should share the same behaviors, since the business concern is the same.

## Decision

**Introduce a typed pipeline of *behaviors* around the existing `Mediator`, and
let a use case opt into it per command. The controller macro keeps calling the
service directly by default; nothing is forced through the bus.** A behavior is
itself a handler that wraps an inner handler — exactly the shape `Instrument` and
`CacheQueries` already use — so behaviors compose by nesting and stay fully typed
and monomorphised. No runtime registry, no `TypeId → Box<dyn Any>`.

The shipped behaviors, and where they sit relative to the transaction boundary:

- **Pre-transaction (fail fast, before touching the database):** `Authorize`
  (reuses the `#[requires]`/`#[public]` policy from the descriptor, ADR 0019),
  `Validate` (the `Valid<T>`/`Validate` contract), `Trace` (the `Instrument`
  seam / a `tracing` span).
- **Transactional scope:** a `Transactional` behavior opens `UnitOfWork::transaction`
  (ADR 0002/0004 — scope-based, closure-scoped) and runs the inner chain *inside*
  it. Because the unit of work is a scope, the behavior receives the rest of the
  chain as the closure body, so the following steps genuinely commit together:
  - **Intra-transaction:** `Idempotent` (`IdempotencyStore::begin`/`finish`,
    ADR 0011), the use-case **handler** (the mutation), and the **outbox publish**
    (`EventPublisher`, written in the same transaction, ADR 0011/CLAUDE.md).

This split is the whole point: the bus does not flatten these into one stack of
middlewares above the handler — it models that some behaviors are *outside* the
transaction (reject early, cheaply) and some are *inside* it (atomic with the
mutation). The order for a write is therefore:

```text
Trace → Authorize → Validate → Transactional{ Idempotent → handler → outbox }
```

A read (`Query`) uses the subset `Trace → Authorize → CacheQueries → handler`;
commands are never cached (`CacheQueries` already passes commands through
untouched).

**Opt-in is the adoption mechanism.** A use case joins the bus by wiring its
handler through the behavior chain at the composition root; a use case that has
not been migrated keeps calling its service directly, exactly as today. The
`#[cached]` and `#[policy]` facade seams become live — they select the
`CacheQueries` / `Trace`+`Authorize` behaviors for that method — instead of being
inert markers.

This also activates, at last, the seams `seiva-cqrs` was written to expose and
closes the gap its own module docs describe ("the hook `#[policy]`/`#[cached]`
will map onto").

## Alternatives considered

- **Make the bus the default in `#[seiva::controller]` (route every use case
  through it).** The maximal consolidation, and the likely end state. Rejected
  *for now*: it rewrites the entire proven HTTP path in one move, immediately
  after a stabilization pass, which is exactly how regressions re-enter. The
  plan is to prove the pipeline on one module (orders) under opt-in, then revisit
  a default in a follow-up ADR once most use cases already run through it.
- **A runtime mediator registry (`TypeId → Box<dyn Any>`, à la MediatR).**
  Rejected, and `seiva-cqrs`'s own docs already argue why: it reintroduces a
  runtime failure mode the type system otherwise rules out, forces boxing and
  dynamic dispatch, and makes middleware composition untyped. It contradicts the
  framework's "everything known at compile time" principle.
- **Keep the concerns as HTTP tower layers (do nothing).** Rejected: tower
  operates on the HTTP `Request`/`Response`, *above* any transaction, so it
  cannot host the idempotency/outbox behaviors that must be atomic with the
  mutation — the same reason ADR 0011 avoided an outer idempotency middleware.
  It is also not transport-agnostic: a gRPC call or an event consumer never
  passes through the HTTP tower stack.
- **A new `#[seiva::command]` macro that generates the handler + wiring.** A
  reasonable ergonomic layer, but it bakes in the wiring before the pipeline has
  been proven. Deferred: start with explicit wiring at the composition root; add
  a macro only once the behavior set and order are settled.

## Consequences

- **Easier:** a use case states its cross-cutting concerns as one typed,
  monomorphised chain; the `#[cached]`/`#[policy]` seams finally do something;
  the same behaviors serve HTTP, gRPC and event consumers because they sit below
  transport.
- **Harder / debt (dated 2026-11-08):** during migration two paths coexist — a
  use case either goes through the bus or calls its service directly — so a reader
  must know which. Accepted deliberately to avoid a big-bang rewrite; the opt-in
  is the price of reversibility. The behavior order relative to the transaction
  boundary (pre- vs intra-transaction) is a correctness invariant that must be
  documented and tested, not left to convention.
- **Enforcement:** one example (orders) is migrated onto the bus as the proof,
  with a test that exercises the full chain (authz → validate → idempotency → tx
  → outbox) and shows one effect under a retried idempotency key; a fitness check
  that dispatch stays static (no `dyn`/`Any` on the use-case path); the existing
  direct-call examples stay green, proving opt-in does not disturb them.

## Related

- [ADR 0002](0002-core-ports-and-test-doubles.md) · [ADR 0003](0003-module-layering-and-facade-impl.md) · [ADR 0006](0006-identity-and-authentication.md) · [ADR 0011](0011-durable-idempotency.md) · [ADR 0019](0019-fail-closed-authorization.md) · [ADR 0021](0021-in-process-event-bus-and-choreography.md)

---

*Part of the [Seiva documentation](../index.md).*
