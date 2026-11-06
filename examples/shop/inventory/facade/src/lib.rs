//! Facade layer of the `inventory` module: the versioned public contract and
//! DTOs.
//!
//! The `inventory` tracks how many units of each product are on hand. It is one
//! of the five independent shop modules; it shares no crate with `catalog` or
//! `orders` — it learns about products only through the `product_id` a caller
//! passes (and, in the microservice story, through events).
//!
//! The generated `#[seiva::controller]` (wired in `shop-inventory-interface`)
//! uses these DTOs as the wire types: the write DTOs derive `serde` and implement
//! [`seiva_http::Validate`], the response DTO derives [`serde::Serialize`], and
//! the error maps to an RFC 9457 [`Problem`] here.

#![allow(async_fn_in_trait)]

use delonix_seiva::{facade, WireSchema};
use seiva_http::{Problem, Validate, ValidationError};
use serde::{Deserialize, Serialize};

/// Command DTO: receive stock for a product (a delivery arriving).
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct ReceiveStock {
    pub product_id: String,
    pub qty: u64,
}

impl Validate for ReceiveStock {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.qty == 0 {
            return Err(ValidationError::new("quantity must be greater than zero"));
        }
        Ok(())
    }
}

/// Command DTO: reserve stock for a product (an order claiming units).
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct Reserve {
    pub product_id: String,
    pub qty: u64,
}

impl Validate for Reserve {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.qty == 0 {
            return Err(ValidationError::new("quantity must be greater than zero"));
        }
        Ok(())
    }
}

/// Query DTO: identify a product. `available` is a write-convention route (its
/// name is neither `get` nor `list`), so this is read as a validated JSON body.
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct ProductId {
    pub id: u128,
}

impl Validate for ProductId {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

/// Response DTO: the stock on hand for a product. Never a domain type.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, WireSchema)]
pub struct StockDto {
    pub product_id: String,
    pub available: u64,
}

/// The `inventory` module's error type. The controller maps it to an RFC 9457
/// problem: a rule violation or an unprocessable request is a `422`, a reference
/// to stock that does not exist a `404`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InventoryError {
    /// A domain rule was violated or the request could not be processed.
    Invalid(String),
    /// Stock referenced by the request does not exist.
    NotFound(String),
}

impl std::fmt::Display for InventoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InventoryError::Invalid(m) | InventoryError::NotFound(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for InventoryError {}

impl From<InventoryError> for Problem {
    fn from(error: InventoryError) -> Self {
        match error {
            InventoryError::Invalid(m) => Problem::unprocessable_entity(m),
            InventoryError::NotFound(m) => Problem::not_found(m),
        }
    }
}

/// The `inventory` module's public contract. Each method declares the permission
/// a caller must hold; the transport enforces it (deny by default, ADR 0005).
#[facade(version = "v1")]
pub trait Inventory {
    /// Receive `qty` units of a product, returning the new on-hand total.
    #[requires("inventory.manage")]
    async fn receive(&self, cmd: ReceiveStock) -> Result<StockDto, InventoryError>;

    /// Reserve `qty` units of a product, returning the remaining on-hand total.
    /// Fails if fewer than `qty` units are available.
    #[requires("inventory.manage")]
    async fn reserve(&self, cmd: Reserve) -> Result<StockDto, InventoryError>;

    /// The units currently available for a product (zero if never stocked).
    #[requires("inventory.read")]
    async fn available(&self, q: ProductId) -> Result<StockDto, InventoryError>;
}
