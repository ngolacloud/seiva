# The tutorial: building a shop

This tutorial builds a **small e-commerce shop** out of **five modules** and shows
the payoff of Seiva's module boundary: the same code runs as **one modular
monolith** and, when you need it, as **independent microservices** — by
composition, not a rewrite. You start with an empty checkout and end with a
backend that scales from one binary to a fleet of services, plus a browser
storefront.

The finished code is
[`examples/shop`](https://github.com/ngolacloud/seiva/tree/main/examples/shop) in
the repository — five five-layer modules that compile and are tested.

## The shop

| Module | Owns | Does |
| --- | --- | --- |
| `catalog` | products | add a product, list, get one |
| `inventory` | stock levels | receive stock, reserve stock, check availability |
| `orders` | orders | place an order (lines of product + quantity), get, list |
| `payments` | payments | charge an order, get a payment |
| `notifications` | sent messages | send a notification, list |

No module calls another module's code. They coordinate through **domain events**:
placing an order emits `orders.placed`; that drives a stock reservation
(`inventory.reserved`), a charge (`payments.captured`), and a confirmation
(`notifications.sent`). Each module owns its data and its contract; the flow is
wiring, not coupling.

## The parts

1. [A modular monolith](part-1-the-modular-monolith.md) — scaffold the shop and its five modules.
2. [Modules own their data](part-2-modules-own-their-data.md) — the five contracts and their permissions.
3. [Events between modules](part-3-events-between-modules.md) — the order flow, wired by events.
4. [The data layer](part-4-the-data-layer.md) — give the shop real tables with DORM.
5. [Authorization across modules](part-5-authorization.md) — who may do what, enforced by the contract.
6. [CQRS](part-6-cqrs.md) — separate reads from writes with `seiva-cqrs`.
7. [Auto-generated API docs & Swagger](part-7-api-docs-and-swagger.md) — every flow documented, no spec written.
8. [Testing the shop](part-8-testing.md) — unit tests, the event flow, fitness functions.
9. [Monolith to microservices](part-9-monolith-to-microservices.md) — split `payments` and `notifications` into their own services.
10. [An admin console](part-10-admin-console.md) — `seiva-console-admin` as an extension, no frontend written.
11. [A separate dxjs frontend, over CORS](part-11-a-separate-dxjs-frontend.md) — a decoupled storefront on its own origin.

Each part is complementary: it says **where the code lives**, the **commands to
build and test** it, and runs under **`seiva start --watch`** live-reload.

---

*Part of the [Seiva documentation](../../index.md).*
