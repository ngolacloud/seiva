//! Facade layer of the `inventory` module: the versioned public contract and
//! DTOs.
//!
//! The `inventory` tracks how many units of each product are on hand. It is one
//! of the five independent shop modules; it shares no crate with `catalog` or
//! `orders` — it learns about products only through the `product_id` a caller
//! passes (and, in the microservice story, through events).

#![allow(async_fn_in_trait)]

use delonix_seiva::{facade, WireSchema};

/// Command DTO: receive stock for a product (a delivery arriving).
#[derive(Debug, Clone, WireSchema)]
pub struct ReceiveStock {
    pub product_id: u128,
    pub qty: u64,
}

/// Command DTO: reserve stock for a product (an order claiming units).
#[derive(Debug, Clone, WireSchema)]
pub struct Reserve {
    pub product_id: u128,
    pub qty: u64,
}

/// Query DTO: identify a product (the `<product_id>` URL capture).
#[derive(Debug, Clone, WireSchema)]
pub struct ProductId {
    pub id: u128,
}

/// Response DTO: the stock on hand for a product. Never a domain type.
#[derive(Debug, Clone, PartialEq, Eq, WireSchema)]
pub struct StockDto {
    pub product_id: u128,
    pub available: u64,
}

/// The `inventory` module's public contract. Each method declares the permission
/// a caller must hold; the transport enforces it (deny by default, ADR 0005).
#[facade(version = "v1")]
pub trait Inventory {
    /// Receive `qty` units of a product, returning the new on-hand total.
    #[requires("inventory.manage")]
    async fn receive(&self, cmd: ReceiveStock) -> Result<StockDto, String>;

    /// Reserve `qty` units of a product, returning the remaining on-hand total.
    /// Fails if fewer than `qty` units are available.
    #[requires("inventory.manage")]
    async fn reserve(&self, cmd: Reserve) -> Result<StockDto, String>;

    /// The units currently available for a product (zero if never stocked).
    #[requires("inventory.read")]
    async fn available(&self, q: ProductId) -> Result<StockDto, String>;
}
