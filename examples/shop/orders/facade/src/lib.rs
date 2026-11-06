//! Facade layer of the `orders` module: the versioned public contract and DTOs.
//!
//! The `orders` module records customer orders, each a list of product lines.
//! It is one of the five independent shop modules; it names products only by
//! `product_id` and shares no crate with `catalog`, `inventory` or `payments`.
//!
//! `PlaceOrder` carries a `Vec<LineDto>`: `WireSchema` supports `Vec<T>` of
//! nested structs, so `LineDto` derives `WireSchema` too.

#![allow(async_fn_in_trait)]

use delonix_seiva::{facade, WireSchema};

/// A single line of an order: a quantity of one product.
#[derive(Debug, Clone, PartialEq, Eq, WireSchema)]
pub struct LineDto {
    pub product_id: u128,
    pub qty: u64,
}

/// Command DTO: place an order for a customer with one or more lines.
#[derive(Debug, Clone, WireSchema)]
pub struct PlaceOrder {
    pub customer: String,
    pub lines: Vec<LineDto>,
}

/// Query DTO: identify an order (the `<order_id>` URL capture).
#[derive(Debug, Clone, WireSchema)]
pub struct OrderId {
    pub id: u128,
}

/// Response DTO for an order. Never a domain type.
#[derive(Debug, Clone, PartialEq, Eq, WireSchema)]
pub struct OrderDto {
    pub id: u128,
    pub customer: String,
    pub lines: Vec<LineDto>,
    pub status: String,
}

/// The `orders` module's public contract. Each method declares the permission a
/// caller must hold; the transport enforces it (deny by default, ADR 0005).
#[facade(version = "v1")]
pub trait Orders {
    /// Place an order and return it with its assigned id and status.
    #[requires("orders.manage")]
    async fn place(&self, cmd: PlaceOrder) -> Result<OrderDto, String>;

    /// A single order by id.
    #[requires("orders.read")]
    async fn get(&self, q: OrderId) -> Result<OrderDto, String>;

    /// Every order placed so far.
    #[requires("orders.read")]
    async fn list(&self) -> Result<Vec<OrderDto>, String>;
}
