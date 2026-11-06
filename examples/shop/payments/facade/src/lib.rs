//! Facade layer of the `payments` module: the versioned public contract and
//! DTOs.
//!
//! The `payments` module charges an order and records the outcome. It is one of
//! the five independent shop modules; it names an order only by `order_id` and
//! shares no crate with `orders`. The charge outcome (captured or declined) is
//! decided in the use case — for the demo a zero amount is declined.

#![allow(async_fn_in_trait)]

use delonix_seiva::{facade, WireSchema};

/// Command DTO: charge an order for an amount.
#[derive(Debug, Clone, WireSchema)]
pub struct Charge {
    pub order_id: u128,
    pub amount_cents: u64,
}

/// Query DTO: identify a payment (the `<payment_id>` URL capture).
#[derive(Debug, Clone, WireSchema)]
pub struct PaymentId {
    pub id: u128,
}

/// Response DTO for a payment, carrying its `captured`/`declined` status. Never
/// a domain type.
#[derive(Debug, Clone, PartialEq, Eq, WireSchema)]
pub struct PaymentDto {
    pub id: u128,
    pub order_id: u128,
    pub amount_cents: u64,
    pub status: String,
}

/// The `payments` module's public contract. Each method declares the permission
/// a caller must hold; the transport enforces it (deny by default, ADR 0005).
#[facade(version = "v1")]
pub trait Payments {
    /// Charge an order. Returns the payment, captured or declined. A charge is
    /// always recorded, so a decline still yields a `PaymentDto`.
    #[requires("payments.charge")]
    async fn charge(&self, cmd: Charge) -> Result<PaymentDto, String>;

    /// A single payment by id.
    #[requires("payments.read")]
    async fn get(&self, q: PaymentId) -> Result<PaymentDto, String>;
}
