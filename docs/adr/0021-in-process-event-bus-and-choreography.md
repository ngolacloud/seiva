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

## Related

- [ADR 0002](0002-core-ports-and-test-doubles.md) · [ADR 0003](0003-module-layering-and-facade-impl.md) · [ADR 0011](0011-durable-idempotency.md) · [ADR 0020](0020-command-bus-and-pipeline-behaviors.md)

---

*Part of the [Seiva documentation](../index.md).*
