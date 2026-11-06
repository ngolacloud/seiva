//! Interface layer of the `catalog` module: the transport-facing shell.
//!
//! Depends on the `facade` trait only; it holds any `Catalog` implementation and
//! exposes it. It never sees `domain` — the application service already mapped
//! the domain entities to DTOs (ADR 0003). In a scaffolded module the
//! `#[seiva::controller]` macro generates the axum routes from the facade; this
//! example keeps a plain wrapper so the layering is provable without the
//! transport runtime.

use shop_catalog_facade::{Catalog, CreateProduct, ProductDto, ProductId};

/// Wraps a `Catalog` implementation and exposes it. The concrete service comes
/// from `application`, chosen by the composition root in `apps/`.
pub struct CatalogApi<S: Catalog> {
    pub service: S,
}

impl<S: Catalog> CatalogApi<S> {
    pub fn new(service: S) -> Self {
        Self { service }
    }

    /// The handler a `POST /v1/catalog/products` route will call.
    pub async fn add_product(&self, name: String, price_cents: u64) -> Result<ProductDto, String> {
        self.service
            .add_product(CreateProduct { name, price_cents })
            .await
    }

    /// The handler the `GET /v1/catalog` index route will call.
    pub async fn list(&self) -> Result<Vec<ProductDto>, String> {
        self.service.list().await
    }

    /// The handler the `GET /v1/catalog/{id}` detail route will call.
    pub async fn get(&self, id: u128) -> Result<ProductDto, String> {
        self.service.get(ProductId { id }).await
    }
}
