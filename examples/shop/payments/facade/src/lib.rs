//! Facade layer of the `payments` module: the versioned public contract and
//! DTOs.
//!
//! The `payments` module charges an order and records the outcome. It is one of
//! the five independent shop modules; it names an order only by `order_id` and
//! shares no crate with `orders`. The charge outcome (captured or declined) is
//! decided in the use case — for the demo a zero amount is declined.
//!
//! The generated `#[seiva::controller]` (wired in `shop-payments-interface`) uses
//! these DTOs as the wire types: the write DTO derives `serde` and implements
//! [`seiva_http::Validate`], the response DTO derives [`serde::Serialize`], and
//! the error maps to an RFC 9457 [`Problem`] here.

#![allow(async_fn_in_trait)]

use delonix_seiva::{facade, WireSchema};
use seiva_http::{Problem, Validate, ValidationError};
use serde::{Deserialize, Serialize};

/// Command DTO: charge an order for an amount. A zero amount is a valid request
/// (it is *declined*, not rejected), so there is no amount rule to validate here.
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct Charge {
    pub order_id: String,
    pub amount_cents: u64,
}

impl Validate for Charge {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

/// Query DTO: identify a payment (the `{id}` URL capture on the `get` route).
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct PaymentId {
    pub id: u128,
}

/// Response DTO for a payment, carrying its `captured`/`declined` status. Never
/// a domain type.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, WireSchema)]
pub struct PaymentDto {
    pub id: String,
    pub order_id: String,
    pub amount_cents: u64,
    pub status: String,
}

/// The `payments` module's error type. A rule violation is a `422`, a missing
/// payment a `404`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentsError {
    /// A domain rule was violated or the request could not be processed.
    Invalid(String),
    /// No payment exists for the requested id.
    NotFound(String),
}

impl std::fmt::Display for PaymentsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaymentsError::Invalid(m) | PaymentsError::NotFound(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for PaymentsError {}

impl From<PaymentsError> for Problem {
    fn from(error: PaymentsError) -> Self {
        match error {
            PaymentsError::Invalid(m) => Problem::unprocessable_entity(m),
            PaymentsError::NotFound(m) => Problem::not_found(m),
        }
    }
}

/// The `payments` module's public contract. Each method declares the permission
/// a caller must hold; the transport enforces it (deny by default, ADR 0005).
#[facade(version = "v1")]
pub trait Payments {
    /// Charge an order. Returns the payment, captured or declined. A charge is
    /// always recorded, so a decline still yields a `PaymentDto`.
    #[requires("payments.charge")]
    async fn charge(&self, cmd: Charge) -> Result<PaymentDto, PaymentsError>;

    /// A single payment by id. The parameter is named `id` so the generated
    /// OpenAPI path (`/v1/payments/{id}`) matches the served route.
    #[requires("payments.read")]
    async fn get(&self, id: PaymentId) -> Result<PaymentDto, PaymentsError>;
}
