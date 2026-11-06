//! Facade layer of the `catalog` module: the versioned public contract and DTOs.
//!
//! The `catalog` is the shop's product list. It is one of the five independent
//! modules the shop tutorial composes (catalog, inventory, orders, payments,
//! notifications); none depends on another's crates — the monolith→microservice
//! story is told through composition and events, not coupling.
//!
//! Like every Seiva module it exposes **one machine-readable contract**
//! (`#[facade]`); the HTTP routes, the OpenAPI document and the gRPC service are
//! all derived from it.

#![allow(async_fn_in_trait)]

use delonix_seiva::{facade, WireSchema};

/// Command DTO: add a product to the catalog.
#[derive(Debug, Clone, WireSchema)]
pub struct CreateProduct {
    pub name: String,
    pub price_cents: u64,
}

/// Query DTO: identify a product (the `<product_id>` URL capture).
#[derive(Debug, Clone, WireSchema)]
pub struct ProductId {
    pub id: u128,
}

/// Response DTO for a product. Never a domain type.
#[derive(Debug, Clone, PartialEq, Eq, WireSchema)]
pub struct ProductDto {
    pub id: u128,
    pub name: String,
    pub price_cents: u64,
}

/// The `catalog` module's public contract. Each method declares the permission a
/// caller must hold; the transport enforces it (deny by default, ADR 0005).
#[facade(version = "v1")]
pub trait Catalog {
    /// Add a product to the catalog.
    #[requires("catalog.manage")]
    async fn add_product(&self, cmd: CreateProduct) -> Result<ProductDto, String>;

    /// Every product in the catalog.
    #[requires("catalog.read")]
    async fn list(&self) -> Result<Vec<ProductDto>, String>;

    /// A single product by id.
    #[requires("catalog.read")]
    async fn get(&self, q: ProductId) -> Result<ProductDto, String>;
}
