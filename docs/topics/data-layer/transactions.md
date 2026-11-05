# Transactions and the unit of work

A use case that changes data does so inside a **transaction**, through the
`UnitOfWork` port — so the application layer expresses atomicity without naming the
database.

## The `UnitOfWork` port

`seiva-ports` defines a scope-based unit of work: the application runs its changes
inside `transaction(|| async { … })`. The body returning `Ok` commits; returning
`Err` rolls back. The application depends only on the trait; the DORM gateway
implements it.

```rust
unit_of_work.transaction(|| async {
    repository.save(&record).await?;          // business change
    events.publish("notes.created", &data).await?;  // event, same tx
    Ok(())
}).await?;
```

## The DORM-backed unit of work

The gateway maps the port onto DORM's closure-scoped executor: `with_session`
(a per-module session — strict mode requires one) plus `db::atomic` (a real
database transaction). The boundary error is bridged to a DORM error so `atomic`
rolls back, and back to the port's `BoxError` on the way out (ADR 0004). Proven
live: a committing transaction persists; a failing one leaves nothing behind.

## The transactional outbox

The `EventPublisher` port, when DORM-backed, writes the event to a transactional
**outbox** *in the same transaction* as the business change — so the event and the
change commit together or not at all (no lost or phantom events). A relay worker
forwards outbox events to the broker afterwards. This is why `publish` must be
called inside `transaction`; see [messaging and events](../messaging-and-events.md).

## Interactive vs scope-based

The port is scope-based (a closure) rather than a handle you pass around, which is
what makes the application layer testable (a fake unit of work just runs the
body) and keeps the transaction boundary explicit and lexical.

## Related

- [Making queries](queries.md) · [Messaging and events](../messaging-and-events.md)
- [The data layer](index.md) · [ADR 0004 — DORM gateway](../../adr/0004-dorm-data-layer-gateway.md)

---

*Part of the [Seiva documentation](../../index.md).*
