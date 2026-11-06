# Part 6: CQRS — separating reads from writes

A write that places an order and a read that lists orders have different shapes,
different costs and different caching rules. **CQRS** (Command/Query Responsibility
Segregation) makes that split explicit: a **command** changes state, a **query**
reads it, and a **mediator** routes each message to its handler. Seiva ships it as
`seiva-cqrs` — statically typed and zero-cost (the mediator is monomorphised over
the handler it holds, so there is no dynamic dispatch).

You do not need CQRS for simple CRUD. Reach for it when the read and write paths
diverge — which, for `orders`, they do: placing an order runs a transaction and
emits an event, while reading one is a cheap lookup worth caching.

## The command and the query

In `examples/shop/orders/application/src/cqrs.rs` the orders module exposes a
`PlaceOrder` command and a `GetOrder` query, each carried out by one handler:

```rust
use seiva_cqrs::{Command, Query, Mediator, Instrument, CacheQueries};

// A command changes state; its Output is what the handler returns.
pub struct PlaceOrderCommand { pub customer: String, pub lines: Vec<Line> }
impl Command for PlaceOrderCommand { type Output = OrderDto; }

// A query reads state; it is cacheable by key.
pub struct GetOrder { pub id: u128 }
impl Query for GetOrder { type Output = OrderDto; }
```

A single `OrdersHandler` implements `CommandHandler<PlaceOrderCommand>` and
`QueryHandler<GetOrder>` over the module's `OrdersService` — the same use case from
Part 1, now reachable through the bus.

## Wiring the mediator with typed middleware

The mediator is built once, wrapping the handler in typed middleware — here
instrumentation plus a read-through cache for the `GetOrder` query:

```rust
let mediator = Mediator::new(
    Instrument::new(
        CacheQueries::<_, GetOrder>::new(OrdersHandler::new(service)),
    ),
);

let placed = mediator.send(PlaceOrderCommand { customer, lines }).await?; // command
let fetched = mediator.query(GetOrder { id: placed.id }).await?;          // query (cached)
```

`CacheQueries` memoises the read side only — a second `GetOrder` for the same id is
served from the cache, never re-running the handler; a `PlaceOrder` always runs and
is never cached. Dispatch is resolved at compile time: a message with no registered
handler is a *compile error*, not a runtime surprise.

## Where the code lives

```text
modules/orders/
├── application/
│   ├── src/lib.rs          # the OrdersService use case (Part 1)
│   └── src/cqrs.rs         # ← PlaceOrderCommand, GetOrder, OrdersHandler, the mediator
```

CQRS stays in the **application** layer; `seiva-cqrs` is an application dependency,
and the layer still names no transport or framework entrypoint (`seiva lint`
enforces this).

## Build and test

```console
$ cargo test -p shop-orders-application           # runs the CQRS tests
$ cargo run -p seiva-cli --bin seiva -- lint      # the layering stays honest
```

The module's tests send the command through the mediator and query it back,
asserting the cache is hit (the second `GetOrder` does not re-run the handler) and
that an invalid command fails through the bus.

## Live-reload while you work

```console
$ seiva start --watch        # rebuilds + restarts on save (needs cargo-watch)
```

## Next

[Part 7: Auto-generated API docs & Swagger](part-7-api-docs-and-swagger.md) — see
every command and query show up in the OpenAPI document and Swagger UI, with no
spec written by hand.

---

*Part of the [Seiva documentation](../../index.md).*
