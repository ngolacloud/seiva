//! Infrastructure layer of the `inventory` module: adapters that implement the
//! application's outbound ports.
//!
//! Depends on `application` + `domain` + `seiva-ports`. In-memory here so the
//! wiring is provable without a database; the real module is DORM-backed.

use seiva_ports::PortResult;
use shop_inventory_application::StockRepository;
use shop_inventory_domain::StockItem;
use std::sync::Mutex;

/// An in-memory [`StockRepository`]. Replace with the DORM-backed adapter.
#[derive(Default)]
pub struct InMemoryStockRepository {
    stock: Mutex<Vec<StockItem>>,
}

impl InMemoryStockRepository {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every stock record stored so far — for composition-level assertions.
    pub fn items(&self) -> Vec<StockItem> {
        self.stock.lock().expect("repository poisoned").clone()
    }
}

impl StockRepository for InMemoryStockRepository {
    async fn get_stock(&self, product_id: u128) -> PortResult<Option<StockItem>> {
        Ok(self
            .stock
            .lock()
            .map_err(|_| "repository lock poisoned")?
            .iter()
            .find(|s| s.product_id == product_id)
            .cloned())
    }

    async fn save_stock(&self, stock: &StockItem) -> PortResult<()> {
        let mut all = self.stock.lock().map_err(|_| "repository lock poisoned")?;
        match all.iter_mut().find(|s| s.product_id == stock.product_id) {
            Some(existing) => *existing = stock.clone(),
            None => all.push(stock.clone()),
        }
        Ok(())
    }
}
