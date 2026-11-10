//! Infrastructure layer of the `orders` module: adapters that implement the
//! application's outbound ports.
//!
//! Depends on `application` + `domain` + `seiva-ports`. In-memory here so the
//! wiring is provable without a database; the real module is DORM-backed.

use shop_orders_domain::{Order, OrderRepository, RepositoryError};
use std::sync::Mutex;

/// An in-memory [`OrderRepository`]. Replace with the DORM-backed adapter.
#[derive(Default)]
pub struct InMemoryOrderRepository {
    orders: Mutex<Vec<Order>>,
}

impl InMemoryOrderRepository {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every order stored so far — for composition-level assertions.
    pub fn orders(&self) -> Vec<Order> {
        self.orders.lock().expect("repository poisoned").clone()
    }
}

impl OrderRepository for InMemoryOrderRepository {
    async fn save_order(&self, order: &Order) -> Result<(), RepositoryError> {
        self.orders
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .push(order.clone());
        Ok(())
    }

    async fn get_order(&self, id: u128) -> Result<Option<Order>, RepositoryError> {
        Ok(self
            .orders
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .iter()
            .find(|o| o.id == id)
            .cloned())
    }

    async fn list_orders(&self) -> Result<Vec<Order>, RepositoryError> {
        Ok(self
            .orders
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .clone())
    }
}
