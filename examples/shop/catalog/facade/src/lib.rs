//! Facade layer of the `catalog` module: the versioned public contract and DTOs.
//!
//! The `catalog` is the shop's product list. It is one of the five independent
//! modules the shop tutorial composes (catalog, inventory, orders, payments,
//! notifications); none depends on another's crates — the monolith→microservice
//! story is told through composition and events, not coupling.
//!
//! Like every Seiva module it exposes **one machine-readable contract**
//! (`#[facade]`); the HTTP routes, the OpenAPI document and the gRPC service are
//! all derived from it. The generated `#[seiva::controller]` (wired in
//! `shop-catalog-interface`) uses these DTOs as the wire types, so the write DTOs
//! derive `serde` and implement [`seiva_http::Validate`], the response DTO
//! derives [`serde::Serialize`], and the error maps to an RFC 9457 [`Problem`]
//! here — hence this layer depends on `delonix-seiva`, `serde` and `seiva-http`.

#![allow(async_fn_in_trait)]

use delonix_seiva::{facade, WireSchema};
use seiva_http::{Problem, Validate, ValidationError};
use serde::{Deserialize, Serialize};

/// Command DTO: add a product to the catalog. Validated by the controller's
/// `Valid<T>` extractor before the use case runs.
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct CreateProduct {
    pub name: String,
    pub price_cents: u64,
}

impl Validate for CreateProduct {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError::new("product name must not be empty"));
        }
        if self.price_cents == 0 {
            return Err(ValidationError::new(
                "product price must be greater than zero",
            ));
        }
        Ok(())
    }
}

/// Query DTO: identify a product (the `{id}` URL capture on the `get` route).
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct ProductId {
    pub id: u128,
}

/// Response DTO for a product. Never a domain type.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, WireSchema)]
pub struct ProductDto {
    pub id: u128,
    pub name: String,
    pub price_cents: u64,
}

/// The `catalog` module's error type. The controller maps it to an RFC 9457
/// problem via this `Into<Problem>`, so the module owns how its failures surface:
/// a domain-rule violation is a `422 Unprocessable Entity`, a missing product a
/// `404 Not Found`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// A domain rule was violated (empty name, zero price, …).
    Invalid(String),
    /// No product exists for the requested id.
    NotFound(String),
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CatalogError::Invalid(m) | CatalogError::NotFound(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for CatalogError {}

impl From<CatalogError> for Problem {
    fn from(error: CatalogError) -> Self {
        match error {
            CatalogError::Invalid(m) => Problem::unprocessable_entity(m),
            CatalogError::NotFound(m) => Problem::not_found(m),
        }
    }
}

/// The `catalog` module's public contract. Each method declares the permission a
/// caller must hold; the transport enforces it (deny by default, ADR 0005).
#[facade(version = "v1")]
pub trait Catalog {
    /// Add a product to the catalog.
    #[requires("catalog.manage")]
    async fn add_product(&self, cmd: CreateProduct) -> Result<ProductDto, CatalogError>;

    /// Every product in the catalog.
    #[requires("catalog.read")]
    async fn list(&self) -> Result<Vec<ProductDto>, CatalogError>;

    /// A single product by id. The parameter is named `id` so the generated
    /// OpenAPI path (`/v1/catalogs/{id}`) matches the served route.
    #[requires("catalog.read")]
    async fn get(&self, id: ProductId) -> Result<ProductDto, CatalogError>;
}
