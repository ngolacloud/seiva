# Messaging and events

A module emits domain events through the `EventPublisher` port; the framework
delivers them reliably via a transactional outbox and a relay worker.

## The `EventPublisher` port

The application publishes an event by topic and payload — it knows nothing of
brokers or envelopes:

```rust
events.publish("notes.created", &serde_json::to_vec(&data)?).await?;
```

Like every port, it is a trait: the application is testable with a fake, and the
delivery mechanism is an adapter.

## CloudEvents envelope

`seiva-messaging` wraps the event data in a **CloudEvents** envelope (id, source,
type, data) — the interchange shape consumers and the relay understand. The
`type` is the topic; the gateway adds the id and source at the boundary.

## Reliable delivery: the transactional outbox

The DORM-backed publisher writes the event to a transactional **outbox** in the
*same database transaction* as the business change ([transactions](data-layer/transactions.md)).
So the change and its event commit atomically — never a change without its event,
never an event for a change that rolled back. That is why `publish` must run
inside a `UnitOfWork::transaction`; the outbox refuses a publish outside one.

A separate **relay worker** (`seiva-outbox-relay`) drains the outbox and forwards
each event to the broker, tracking its own cursor and retrying — so a broker
outage delays delivery but never loses an event.

## Broker backends

Concrete brokers are feature-gated adapters behind the publisher/relay; the
default development path logs, and production wires a real broker. The application
code is unchanged either way.

## Related

- [Transactions and the unit of work](data-layer/transactions.md) · [Background workers](background-workers.md)

---

*Part of the [Seiva documentation](../index.md).*
