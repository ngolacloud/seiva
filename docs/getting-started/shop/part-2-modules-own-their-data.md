# Part 2: Modules own their data

Each module exposes **one contract** — a `#[seiva::facade]` trait whose methods
each declare the permission a caller must hold. The HTTP routes, the OpenAPI and
the permission checks are all derived from it. Here are the five, exactly as they
appear in [`examples/shop`](https://github.com/ngolacloud/seiva/tree/main/examples/shop).

## catalog — products

```rust
#[facade(version = "v1")]
pub trait Catalog {
    #[requires("catalog.manage")]
    async fn add_product(&self, cmd: CreateProduct) -> Result<ProductDto, String>; // catalog.product_added

    #[requires("catalog.read")]
    async fn list(&self) -> Result<Vec<ProductDto>, String>;

    #[requires("catalog.read")]
    async fn get(&self, id: ProductId) -> Result<ProductDto, String>;
}
// CreateProduct { name, price_cents }   ProductDto { id, name, price_cents }
```

The domain rule lives in `catalog-domain`: a product's name must be non-empty and
its `price_cents` must be greater than zero (`Product::new`).

## inventory — stock levels

```rust
#[facade(version = "v1")]
pub trait Inventory {
    #[requires("inventory.manage")]
    async fn receive(&self, cmd: ReceiveStock) -> Result<StockDto, String>;  // inventory.received

    #[requires("inventory.manage")]
    async fn reserve(&self, cmd: Reserve) -> Result<StockDto, String>;       // inventory.reserved

    #[requires("inventory.read")]
    async fn available(&self, id: ProductId) -> Result<StockDto, String>;
}
// ReceiveStock { product_id, qty }   Reserve { product_id, qty }   StockDto { product_id, available }
```

`reserve` fails with an `InsufficientStock` error when the available quantity is
below what is requested — a domain rule, not a database constraint.

## orders — orders

```rust
#[facade(version = "v1")]
pub trait Orders {
    #[requires("orders.manage")]
    async fn place(&self, cmd: PlaceOrder) -> Result<OrderDto, String>;  // orders.placed

    #[requires("orders.read")]
    async fn get(&self, id: OrderId) -> Result<OrderDto, String>;

    #[requires("orders.read")]
    async fn list(&self) -> Result<Vec<OrderDto>, String>;
}
// PlaceOrder { customer, lines: Vec<LineDto> }   LineDto { product_id, qty }
// OrderDto { id, customer, lines, status }   // status starts at "placed"
```

Note `lines` is a `Vec<LineDto>` — a nested wire DTO. Because `LineDto` derives
`WireSchema`, the order's real shape (a list of line items) is published to the
OpenAPI with no hand-written spec. An order must have at least one line and every
line a positive quantity.

## payments — payments

```rust
#[facade(version = "v1")]
pub trait Payments {
    #[requires("payments.charge")]
    async fn charge(&self, cmd: Charge) -> Result<PaymentDto, String>; // payments.captured | payments.declined

    #[requires("payments.read")]
    async fn get(&self, id: PaymentId) -> Result<PaymentDto, String>;
}
// Charge { order_id, amount_cents }   PaymentDto { id, order_id, amount_cents, status }
```

`charge` records a `captured` or `declined` payment (the demo declines a zero
amount); both outcomes commit and emit their event. The decline policy is a
use-case decision, and the domain keeps the invariant that a *captured* payment
can never have a zero amount.

## notifications — sent messages

```rust
#[facade(version = "v1")]
pub trait Notifications {
    #[requires("notifications.send")]
    async fn send(&self, cmd: SendNotification) -> Result<NotificationDto, String>; // notifications.sent

    #[requires("notifications.read")]
    async fn list(&self) -> Result<Vec<NotificationDto>, String>;
}
// SendNotification { to, subject, body }   NotificationDto { id, to, subject, body }
```

!!! note "Why `SendNotification`, not `Send`"
    The command is `SendNotification` rather than the tempting `Send`: `Send` is
    Rust's marker trait, and the application layer's port futures are bounded
    `+ Send`, so a DTO called `Send` would collide with the bound and fail to
    compile. A small reminder that your DTO names share a namespace with the
    standard library.

## Five contracts, no coupling

Each module names only its own types. None of them imports another module's crate.
The next part shows how they still cooperate — through events, not calls.

## Next

[Part 3: Events between modules](part-3-events-between-modules.md).

---

*Part of the [Seiva documentation](../../index.md).*
