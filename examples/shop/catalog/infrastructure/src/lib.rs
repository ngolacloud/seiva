//! Infrastructure layer of the `catalog` module: adapters that implement the
//! application's outbound ports.
//!
//! Depends on `application` + `domain` + `seiva-ports`. In a real module this is
//! the DORM-backed repository; here it is an in-memory adapter so the wiring is
//! provable without a database.

use shop_catalog_domain::{Product, ProductRepository, RepositoryError};
use std::sync::Mutex;

/// An in-memory [`ProductRepository`]. Replace with the DORM-backed adapter.
#[derive(Default)]
pub struct InMemoryProductRepository {
    products: Mutex<Vec<Product>>,
}

impl InMemoryProductRepository {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every product stored so far — for composition-level assertions.
    pub fn products(&self) -> Vec<Product> {
        self.products.lock().expect("repository poisoned").clone()
    }
}

impl ProductRepository for InMemoryProductRepository {
    async fn save_product(&self, product: &Product) -> Result<(), RepositoryError> {
        self.products
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .push(product.clone());
        Ok(())
    }

    async fn list_products(&self) -> Result<Vec<Product>, RepositoryError> {
        Ok(self
            .products
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .clone())
    }

    async fn get_product(&self, id: u128) -> Result<Option<Product>, RepositoryError> {
        Ok(self
            .products
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .iter()
            .find(|p| p.id == id)
            .cloned())
    }
}
