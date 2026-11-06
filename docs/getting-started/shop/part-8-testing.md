# Part 8: Testing the shop

Five modules, and the whole thing still tests without a server or a database. The
shop's modules code against ports, so their use cases run against in-memory fakes
from `seiva-testing` — the same three levels the first tutorial introduced, now
across a system.

## Unit-test each use case

Each module's `application` crate tests the happy path, a rule rejection, and that
the right event was emitted — with fakes for every port. This is the real
`reserve` test from `shop-inventory-application`, condensed:

```rust
#[tokio::test]
async fn reserve_decrements_available_and_emits() {
    let svc = service();
    svc.receive(ReceiveStock { product_id: 1, qty: 10 }).await.unwrap();

    let stock = svc.reserve(Reserve { product_id: 1, qty: 3 }).await.unwrap();
    assert_eq!(stock.available, 7);
    assert_eq!(svc.events.topics(), ["inventory.received", "inventory.reserved"]);
}

#[tokio::test]
async fn reserve_more_than_available_is_refused() {
    let svc = service();
    svc.receive(ReceiveStock { product_id: 1, qty: 2 }).await.unwrap();
    assert!(svc.reserve(Reserve { product_id: 1, qty: 5 }).await.is_err());
    // nothing committed, no reservation event
    assert_eq!(svc.events.topics(), ["inventory.received"]);
}
```

`examples/shop` ships **28 unit tests** across the five modules' domain and
application layers, plus a **composition test per module** that drives the service
through its real in-memory adapter via the facade. Run them all:

```console
$ cargo test
```

## The event flow

The payoff of the events design (Part 3) is that the order flow is testable as a
sequence of recorded events. With a `RecordingEventPublisher` shared across the
modules, a composition test asserts that placing an order produces
`orders.placed`, then `inventory.reserved`, then `payments.captured`, then
`notifications.sent` — in order, with nothing emitted when a step fails (an
out-of-stock order never reaches `payments`).

## Architecture fitness functions

`seiva lint` runs the invariants that keep five modules from quietly growing
dependencies on each other:

```console
$ seiva lint
```

It fails the build if a module's `interface` depends on its `infrastructure`, if an
`application` names `delonix_seiva` directly, or — the one that matters most with
several modules — if one module imports another module's crate. The boundary from
Part 2 is enforced, not hoped for.

## Next

[Part 9: Monolith to microservices](part-9-monolith-to-microservices.md).

---

*Part of the [Seiva documentation](../../index.md).*
