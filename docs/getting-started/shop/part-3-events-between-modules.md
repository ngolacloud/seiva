# Part 3: Events between modules

The shop's modules never call each other. They cooperate through **domain
events**: a module publishes a fact about its own data, and other modules react.
This is what keeps them independent — and what makes the microservice split in
Part 7 possible.

## One use case, one transaction, one event

Every write use case runs inside a single `UnitOfWork::transaction`, and it
publishes its event **inside that transaction**. With the DORM data layer (Part 4)
the event lands in a transactional **outbox** in the *same* transaction as the
business change, so the two commit together or not at all. Here is `orders.place`,
from `shop-orders-application`, condensed:

```rust
self.uow.transaction(|| async {
    self.repository.save(&order).await?;
    self.events.publish("orders.placed", &event).await   // same transaction
}).await?;
```

The application layer depends only on the `EventPublisher` **port** — it does not
know whether the event goes to an in-memory recorder (tests), a log, or a broker.
That decision belongs to the composition root.

## The order flow

Placing an order sets off a chain. Each arrow is one module reacting to another
module's event — never a direct call:

```text
orders.place
   └─▶ orders.placed ──▶ inventory.reserve  ──▶ inventory.reserved
                                              └▶ payments.charge   ──▶ payments.captured
                                                                    └▶ notifications.send ─▶ notifications.sent
```

- `orders` publishes **`orders.placed`** with the order's lines.
- `inventory` reacts: it reserves stock for each line and publishes
  **`inventory.reserved`** (or fails with `InsufficientStock`).
- `payments` reacts to a reserved order: it charges the total and publishes
  **`payments.captured`** (or **`payments.declined`**).
- `notifications` reacts to a captured payment: it sends a confirmation and
  publishes **`notifications.sent`**.

Each module only ever touches its own data. The flow is a sequence of
*facts*, not a call stack.

## Who wires the reactions?

The **composition root** (`apps/shop-api`) owns the wiring. It subscribes each
reacting module to the events it cares about — a small *process manager* that
turns `orders.placed` into a call to `inventory.reserve`, and so on. The modules
stay oblivious; swap the wiring and the same modules behave differently.

This is also the seam for Part 7: when `payments` becomes its own service, nothing
in the `payments` module changes — only *where* the `payments.charge` reaction
runs and *how* the event reaches it (an in-process bus becomes a broker topic).

See [Messaging and events](../../topics/messaging-and-events.md) for the broker
options and the CloudEvents envelope, and
[Background workers](../../topics/background-workers.md) for the outbox relay that
forwards committed events.

## Next

[Part 4: The data layer](part-4-the-data-layer.md).

---

*Part of the [Seiva documentation](../../index.md).*
