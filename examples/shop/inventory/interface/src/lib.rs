//! Interface layer of the `inventory` module: the transport-facing shell.
//!
//! Depends on the `facade` trait only; it holds any `Inventory` implementation
//! and exposes it. It never sees `domain` (ADR 0003). A scaffolded module would
//! let `#[seiva::controller]` generate the axum routes from the facade; this
//! example keeps a plain wrapper so the layering is provable without the
//! transport runtime.

use shop_inventory_facade::{Inventory, ProductId, ReceiveStock, Reserve, StockDto};

/// Wraps an `Inventory` implementation and exposes it.
pub struct InventoryApi<S: Inventory> {
    pub service: S,
}

impl<S: Inventory> InventoryApi<S> {
    pub fn new(service: S) -> Self {
        Self { service }
    }

    /// The handler a `POST /v1/inventory/receipts` route will call.
    pub async fn receive(&self, product_id: u128, qty: u64) -> Result<StockDto, String> {
        self.service.receive(ReceiveStock { product_id, qty }).await
    }

    /// The handler a `POST /v1/inventory/reservations` route will call.
    pub async fn reserve(&self, product_id: u128, qty: u64) -> Result<StockDto, String> {
        self.service.reserve(Reserve { product_id, qty }).await
    }

    /// The handler the `GET /v1/inventory/{id}` route will call.
    pub async fn available(&self, id: u128) -> Result<StockDto, String> {
        self.service.available(ProductId { id }).await
    }
}
