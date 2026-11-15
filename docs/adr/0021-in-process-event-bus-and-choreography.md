# ADR 0021 — An in-process event bus closes the outbox loop; modules choreograph the same way whether monolith or microservice

- Status: Accepted
- Date: 2026-11-08
- Deciders: NgolaCloud
- Follows: ADR 0002 (ports), ADR 0003 (module layering), ADR 0011 (durable idempotency / inbox dedup)

## Context

Seiva's central promise is that **modules cooperate through events, not calls**,
and that the same module code runs as a modular monolith or as a microservice by
*composition alone* — this is what makes `seiva module split` a composition change
rather than a rewrite (CLAUDE.md). The canonical flow is stated there:
`orders.placed → inventory.reserved → payments.captured → notifications.sent`,
wired by the composition root reacting to events.

Half of this is already built and proven:

- A write use case publishes its domain event **inside** its `UnitOfWork`
  transaction through the `EventPublisher` port; with DORM that lands in a
  **transactional outbox** (`DormOutboxPublisher`), so data and event commit
  atomically (ADR 0011, `seiva-data-dorm`).
- The outbox is drained to a broker **at-least-once** by `seiva-outbox-relay`
  (advancing the cursor only after a successful publish), the wire envelope is
  **CloudEvents 1.0** with NATS/AMQP/Kafka backends (`seiva-messaging`), and the
  consumer side deduplicates with an **inbox** key through the same
  `IdempotencyStore` so a redelivery is consumed once (ADR 0011).

The other half is missing. There is **no in-process event bus**: the apps wire
`NoopEventPublisher` (the event is dropped), and **no choreography is mounted** in
any app — the `orders → inventory` flow is aspirational. So a module cannot yet
react to another module's event *in a single process*, which is exactly the
monolith deployment the framework claims to support out of the box.

The forces:

- A module publishes a domain event; another module **reacts without importing its
  crate** — preserving the independence that lets `module split` work.
- In a **monolith** the reaction happens in-process; in **microservices** it
  happens via the broker. The *subscriber's code must be byte-identical* in both,
  or "split is just composition" is false.
- The guarantees already established — publish-inside-the-transaction, at-least-once
  delivery, consume-once via the inbox — must hold for the in-process path too,
  otherwise a monolith would be weaker than its split form.
- Local development must run the choreography **without standing up a broker**.

## Decision

**Add an in-process event bus at the composition root, and make the *only*
difference between monolith and microservice the dispatcher that moves events from
the outbox to subscribers — not the publisher, not the subscriber, not the
guarantees.** The outbox stays the single point of publication in both modes.

- **Publication is unchanged.** A use case keeps publishing through `EventPublisher`
  into the transactional outbox, inside its transaction. We do **not** add a
  direct in-process `publish → call subscriber` path, because that event would not
  be durable (a crash between commit and in-process delivery loses it) and would
  diverge from the broker path.
- **Two dispatchers, one contract.** An event leaves the outbox via one of:
  - **in-process dispatcher** (new): reads the outbox and invokes locally
    registered subscribers, marking each delivery once through the inbox
    (`inbox_key` + `IdempotencyStore`, ADR 0011) — the in-process twin of
    `seiva-outbox-relay::drain`;
  - **relay → broker** (exists): `seiva-outbox-relay` forwards to NATS/AMQP/Kafka,
    and the remote consumer drains with the same inbox dedup.
  Because both read the same outbox and both dedup through the same inbox, a
  subscriber handler is **identical** across the two, and delivery is at-least-once
  with exactly-one *effect* either way.
- **Subscribers are registered at the composition root, by type, not by import.**
  A subscriber is a handler over a domain event (the CloudEvent deserialised into
  its typed payload) that runs inside its **own** `UnitOfWork::transaction` with an
  `inbox_key` claim (effect + mark together, ack after commit) — the consume
  pattern ADR 0011 already defines. A module never names another module's crate;
  the composition root is where `orders.placed` is routed to the inventory
  subscriber.
- **Proof by effect.** Replace `NoopEventPublisher` in one app (the shop) and
  mount `orders.placed → inventory.reserved`, demonstrated first under the
  in-process dispatcher (no broker) and then under the relay→broker path, with the
  same subscriber code and one reservation per order in both.

## Alternatives considered

- **Direct in-process delivery (publish calls the subscriber after commit, no
  outbox).** Simpler, but rejected: it is not durable (an event is lost if the
  process dies after commit but before delivery), and it diverges from the broker
  path, so the subscriber would behave differently in a monolith than after a
  split — breaking the framework's core promise.
- **Let modules import and call each other (orders calls inventory directly).**
  Rejected: it reintroduces the compile-time coupling that `module split` exists
  to avoid; the whole point is that cooperation survives being pulled into separate
  processes unchanged.
- **Run a broker even in a monolith (no in-process path at all).** Rejected: it
  forces every single-process deployment and every local `cargo run` to stand up
  NATS/Kafka, contradicting "a monolith by composition" and hurting the developer
  loop.
- **A typed in-process bus with a `TypeId → Box<dyn Any>` subscriber map.** Some
  runtime indirection at the *composition boundary* is unavoidable — subscribers
  are plugged in at the root and the dispatcher deserialises each CloudEvent into
  the subscriber's concrete type. We accept that indirection, but **confine it to
  the composition/dispatch edge**: it never touches a use case's own dispatch path,
  which stays statically typed (contrast ADR 0020). The subscriber handler itself
  is a normal typed handler.

## Consequences

- **Easier:** the framework's stated principle — modules cooperate through events —
  becomes real and proven, not aspirational; `module split` becomes genuinely a
  swap of dispatcher (in-process ↔ relay+broker); local development runs the full
  choreography with no infrastructure.
- **Harder / debt (dated 2026-11-08):** the in-process dispatcher that reads the
  outbox and fans out to subscribers is new code (close kin to the existing relay);
  delivery ordering, retry/back-off and back-pressure for the in-process path must
  be specified, not assumed. Subscriber registration at the composition root is a
  runtime wiring step (accepted, confined to the edge as above).
- **Enforcement:** a test that the *same* subscriber handler produces one effect
  per event under both the in-process dispatcher and the broker drain; a live proof
  of `orders.placed → inventory.reserved` in the shop app; a fitness check that no
  module crate depends on another module crate (the independence `module split`
  relies on).

## Addendum — Fase 2: durability (transactional dispatch, retry/back-off, dead-letter)

- Date: 2026-11-13
- Status: Accepted (extends the decision above; changes the public signatures of
  `seiva_events::dispatch` / `drain_in_process` / `run_in_process_dispatch_worker`
  and adds one defaulted method to the `IdempotencyStore` port)

The Fase 1 dispatcher proved the choreography offline but left three gaps the
"Harder / debt (2026-11-08)" note anticipated — retry, back-off and back-pressure
were "specified, not assumed". Fase 2 specifies them, and closes a latent
correctness bug.

**The bug.** Fase 1 `dispatch` claims the inbox key (`begin` → `Fresh`), runs the
handler, then `finish`es — with **no transaction around the three**. If the handler
errors after a `Fresh` claim, the error propagates and the driver leaves the event
pending (good), but the claim is now stuck half-done: the next redelivery sees
`InProgress`, performs **no** effect, returns `Ok`, and the driver **advances the
cursor past the event** — silently dropping it. A transient failure thus loses the
event. The broker path never had this bug because its claim and effect already share
one `db::atomic` transaction (`seiva-outbox-relay::drain`), so a failed handler rolls
the claim back and the retry is `Fresh` again.

**Decision.**

1. **`dispatch` runs `claim → handle → finish` inside `UnitOfWork::transaction`**, one
   transaction per subscriber (the consume pattern ADR 0011 already defines, now made
   real for the in-process path). A new required `uow: &U` parameter carries it. With
   the DORM gateway the claim, the handler's writes and the mark commit atomically and
   a rollback un-claims the key for free — exactly the broker path's guarantee. The
   transaction runs under `Scope::Global`: the inbox claim is the driver's own
   infrastructure bookkeeping (the `scoped_key` already folds tenant + subscriber +
   event per ADR 0011), and the `CloudEvent` envelope carries no tenant in Fase 2
   (per-event tenant scoping is deferred — see below).
2. **`IdempotencyStore` gains a defaulted `release(scoped_key)`.** A store whose
   `begin` joins the caller's transaction (DORM) un-claims on rollback automatically,
   so its `release` is the default no-op. A store that does **not** participate in the
   `UnitOfWork` (the in-memory double, and any store a user wires without a shared
   transaction) overrides it to drop an unfinished claim. `dispatch` calls `release`
   only for a claim **it** took `Fresh` whose transaction then failed — never a
   `Replay`/`InProgress`/`Mismatch` key it did not own — so no concurrent claim is
   disturbed. This keeps "a rollback un-claims the key" true for every store, not only
   DORM, with a single generic `dispatch` body.
3. **`drain_in_process` dead-letters a poison event instead of blocking the queue.** A
   `RetryPolicy { max_attempts, base_backoff }` governs per-event, in-pass retries with
   exponential back-off; an event that still fails after `max_attempts` is handed to a
   `DeadLetterSink` and the cursor is advanced **past** it, so the next event
   progresses. Without this, Decision 1 (which makes retries actually re-run) would let
   one permanently-failing event retry forever and starve every later event — trading a
   silent drop for a silent stall. `LoggingDeadLetterSink` is the production default;
   `InMemoryDeadLetterQueue` (behind the `testing` feature) makes the divert observable
   offline.

**Alternatives considered (Fase 2).**

- *Explicit `release` always called by `dispatch` on any error.* Rejected: it would
  also fire on `Mismatch` (a key `dispatch` never claimed) and could race a concurrent
  `InProgress` claim of the same key. Gating `release` on "we got `Fresh` this
  transaction" is precise and race-free.
- *A task-local transaction journal coupling the in-memory UoW and store* (mirroring
  DORM's ambient session). Rejected as heavier and as changing the behaviour of every
  existing `InMemoryUnitOfWork` test; the defaulted `release` is local to the one path
  that needs it.
- *Cross-pass durable attempt counting + timed back-off* (a poison event retried across
  worker passes rather than within one). Deferred: it needs a durable per-event attempt
  store; in-pass bounded retry + dead-letter already satisfies "the poison event stops
  blocking the following ones" and keeps the driver stateless. The broker path keeps its
  own retry semantics.

**Consequences (Fase 2).**

- **Easier:** a transient handler failure is now genuinely retried (not dropped); a
  permanent failure is quarantined (not a stall); the monolith's durability equals the
  broker path's.
- **Harder / debt (dated 2026-11-13):** in-pass back-off sleeps block the single drain
  loop while one event waits — acceptable for the in-process driver's scale, revisited
  if a deployment needs concurrent per-event retry. The `CloudEvent` envelope still
  carries no tenant, so `dispatch`'s transaction is `Scope::Global`; adding a tenant to
  the envelope and scoping per event is future work.
- **Enforcement:** offline tests that (a) a handler that fails once is re-run `Fresh` on
  redelivery (the un-claim), and (b) an always-failing event lands in the
  `InMemoryDeadLetterQueue` while the next event processes; plus the live shop app
  draining via `run_in_process_dispatch_worker` mounted in `build_app`.

## Related

- [ADR 0002](0002-core-ports-and-test-doubles.md) · [ADR 0003](0003-module-layering-and-facade-impl.md) · [ADR 0011](0011-durable-idempotency.md) · [ADR 0020](0020-command-bus-and-pipeline-behaviors.md)

---

*Part of the [Seiva documentation](../index.md).*
