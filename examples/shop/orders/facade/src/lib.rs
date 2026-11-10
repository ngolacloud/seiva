//! Facade layer of the `orders` module: the versioned public contract and DTOs.
//!
//! The `orders` module records customer orders, each a list of product lines.
//! It is one of the five independent shop modules; it names products only by
//! `product_id` and shares no crate with `catalog`, `inventory` or `payments`.
//!
//! `PlaceOrder` carries a `Vec<LineDto>`: `WireSchema` supports `Vec<T>` of
//! nested structs, so `LineDto` derives `WireSchema` too. The generated
//! `#[seiva::controller]` (wired in `shop-orders-interface`) uses these DTOs as
//! the wire types: the write DTO derives `serde` and implements
//! [`seiva_contract::Validate`], the response DTO derives [`serde::Serialize`], and
//! the error maps to an RFC 9457 [`Problem`] here.

#![allow(async_fn_in_trait)]

use axum::http::StatusCode;
use delonix_seiva::{facade, WireId, WireSchema};
use seiva_contract::{Problem, Validate, ValidationError};
// `Ctx` (the request context: principal, tenant, idempotency key) is a transport
// concept and stays in `seiva-http`; a facade opts into it only when the use case
// needs the request context. The error/validation contract is neutral (F1).
use seiva_http::Ctx;
use serde::{Deserialize, Serialize};

/// A single line of an order: a quantity of one product.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, WireSchema)]
pub struct LineDto {
    pub product_id: WireId,
    pub qty: u64,
}

/// Command DTO: place an order for a customer with one or more lines.
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct PlaceOrder {
    pub customer: String,
    pub lines: Vec<LineDto>,
}

impl Validate for PlaceOrder {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.customer.trim().is_empty() {
            return Err(ValidationError::new("customer must not be empty"));
        }
        if self.lines.is_empty() {
            return Err(ValidationError::new("an order must have at least one line"));
        }
        if self.lines.iter().any(|l| l.qty == 0) {
            return Err(ValidationError::new(
                "every order line must have a quantity greater than zero",
            ));
        }
        Ok(())
    }
}

/// Query DTO: identify an order (the `{id}` URL capture on the `get` route).
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct OrderId {
    pub id: u128,
}

/// Response DTO for an order. Never a domain type.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, WireSchema)]
pub struct OrderDto {
    pub id: WireId,
    pub customer: String,
    pub lines: Vec<LineDto>,
    pub status: String,
}

/// The `orders` module's error type. A rule violation is a `422`, a missing order
/// a `404`, and an idempotency conflict a `409`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrdersError {
    /// A domain rule was violated (no lines, zero-quantity line, …).
    Invalid(String),
    /// No order exists for the requested id.
    NotFound(String),
    /// An idempotency conflict surfaced by the command bus (ADR 0011/0020): an
    /// `Idempotency-Key` reused with different content, or whose first request is
    /// still in flight. Maps to a `409`. Carried as a distinct variant so the
    /// status the `Idempotent` behavior produced survives the round trip back from
    /// the bus — the controller recovers it and re-maps it here rather than
    /// flattening it to a generic `422`.
    Conflict(String),
}

impl std::fmt::Display for OrdersError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrdersError::Invalid(m) | OrdersError::NotFound(m) | OrdersError::Conflict(m) => {
                f.write_str(m)
            }
        }
    }
}

impl std::error::Error for OrdersError {}

impl From<OrdersError> for Problem {
    fn from(error: OrdersError) -> Self {
        match error {
            OrdersError::Invalid(m) => Problem::unprocessable_entity(m),
            OrdersError::NotFound(m) => Problem::not_found(m),
            OrdersError::Conflict(m) => {
                Problem::new(StatusCode::CONFLICT, "Conflict").with_detail(m)
            }
        }
    }
}

/// The `orders` module's public contract. Each method declares the permission a
/// caller must hold; the transport enforces it (deny by default, ADR 0005).
#[facade(version = "v1")]
pub trait Orders {
    /// Place an order and return it with its assigned id and status.
    ///
    /// Takes the request [`Ctx`] (the caller's principal, the operation
    /// coordinates and the optional `Idempotency-Key`) so the implementation can
    /// dispatch the write through the ADR 0020 command bus — the `Idempotent`
    /// behavior folds the key and body out of the `Ctx`. The `#[controller]` macro
    /// recognises the `&Ctx` parameter and supplies it from the request, so it is
    /// **not** part of the wire contract: the route stays `POST /v1/orders/place`
    /// with the same `PlaceOrder` body.
    #[requires("orders.manage")]
    async fn place(&self, ctx: &Ctx, cmd: PlaceOrder) -> Result<OrderDto, OrdersError>;

    /// A single order by id. The parameter is named `id` so the generated
    /// OpenAPI path (`/v1/orders/{id}`) matches the served route.
    #[requires("orders.read")]
    async fn get(&self, id: OrderId) -> Result<OrderDto, OrdersError>;

    /// Every order placed so far.
    #[requires("orders.read")]
    async fn list(&self) -> Result<Vec<OrderDto>, OrdersError>;
}
