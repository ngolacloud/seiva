//! Domain layer of the `catalog` module: the `Product` entity, its rules AND
//! the repository port.
//!
//! Depends on nothing but `shared/kernel` and std. The rule — a product must
//! carry a non-empty name and a price above zero — lives here, and so does the
//! REPOSITORY PORT: classic DDD puts the repository interface in the domain (its
//! ubiquitous language), framed in a domain error, not the framework's (ADR
//! 0003). The application adapts it.

use std::future::Future;

/// A product in the shop's catalog. Construct it through [`Product::new`] so the
/// rule is always enforced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Product {
    pub id: u128,
    pub name: String,
    pub price_cents: u64,
}

impl Product {
    /// Creates a product, rejecting an empty name or a zero price.
    pub fn new(id: u128, name: impl Into<String>, price_cents: u64) -> Result<Self, DomainError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DomainError::EmptyName);
        }
        if price_cents == 0 {
            return Err(DomainError::ZeroPrice);
        }
        Ok(Self {
            id,
            name,
            price_cents,
        })
    }
}

/// A domain rule was violated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    EmptyName,
    ZeroPrice,
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomainError::EmptyName => write!(f, "product name must not be empty"),
            DomainError::ZeroPrice => write!(f, "product price must be greater than zero"),
        }
    }
}

impl std::error::Error for DomainError {}

/// A repository operation failed. A DOMAIN-level error: the repository is part of
/// the domain's language, so it speaks in domain terms, not the framework's. The
/// application maps it to `seiva_ports::PortResult` at the use-case boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryError(pub String);

impl std::fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RepositoryError {}

/// Outbound port: persistence of products, in the domain's language (classic DDD
/// — the repository interface lives in the domain, ADR 0003). Implemented in
/// `infrastructure` (over DORM or an external library); the application adapts
/// its [`RepositoryError`] to the framework's `PortResult`. Each future is `Send`
/// so a use case can `.await` it inside a `UnitOfWork::transaction` on a
/// multi-threaded runtime.
pub trait ProductRepository {
    fn save_product(
        &self,
        product: &Product,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    fn list_products(&self) -> impl Future<Output = Result<Vec<Product>, RepositoryError>> + Send;

    fn get_product(
        &self,
        id: u128,
    ) -> impl Future<Output = Result<Option<Product>, RepositoryError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_name() {
        assert_eq!(Product::new(1, "   ", 500), Err(DomainError::EmptyName));
    }

    #[test]
    fn rejects_zero_price() {
        assert_eq!(Product::new(1, "Teapot", 0), Err(DomainError::ZeroPrice));
    }

    #[test]
    fn accepts_a_valid_product() {
        let p = Product::new(1, "Teapot", 1299).unwrap();
        assert_eq!((p.id, p.price_cents), (1, 1299));
    }
}
