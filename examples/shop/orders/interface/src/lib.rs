//! Interface layer of the `orders` module: the transport-facing shell.
//!
//! Depends on the `facade` trait only; it holds any `Orders` implementation and
//! exposes it. It never sees `domain` (ADR 0003). A scaffolded module would let
//! `#[seiva::controller]` generate the axum routes from the facade; this example
//! keeps a plain wrapper so the layering is provable without the transport
//! runtime.

use shop_orders_facade::{LineDto, OrderDto, OrderId, Orders, PlaceOrder};

/// Wraps an `Orders` implementation and exposes it.
pub struct OrdersApi<S: Orders> {
    pub service: S,
}

impl<S: Orders> OrdersApi<S> {
    pub fn new(service: S) -> Self {
        Self { service }
    }

    /// The handler a `POST /v1/orders` route will call.
    pub async fn place(&self, customer: String, lines: Vec<LineDto>) -> Result<OrderDto, String> {
        self.service.place(PlaceOrder { customer, lines }).await
    }

    /// The handler the `GET /v1/orders/{id}` route will call.
    pub async fn get(&self, id: u128) -> Result<OrderDto, String> {
        self.service.get(OrderId { id }).await
    }

    /// The handler the `GET /v1/orders` index route will call.
    pub async fn list(&self) -> Result<Vec<OrderDto>, String> {
        self.service.list().await
    }
}
