//! Domain layer of the `inventory` module: the `StockItem` entity, its rules AND
//! the repository port.
//!
//! Depends on nothing but `shared/kernel`. The rules — a received or reserved
//! quantity must be positive, and a reservation can never exceed what is on
//! hand — live here, not in the application or the database. So does the
//! REPOSITORY PORT: classic DDD puts the repository interface in the domain (its
//! ubiquitous language), framed in a domain error, not the framework's (ADR
//! 0003). The application adapts it.

use std::future::Future;

/// The stock on hand for a single product. Construct it through
/// [`StockItem::new`]; mutate it only through [`StockItem::receive`] and
/// [`StockItem::reserve`], which enforce the rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockItem {
    pub product_id: u128,
    pub available: u64,
}

impl StockItem {
    /// Creates an empty stock record for a product (zero units on hand).
    pub fn new(product_id: u128) -> Self {
        Self {
            product_id,
            available: 0,
        }
    }

    /// Adds `qty` units to the on-hand total, rejecting a zero quantity.
    pub fn receive(&mut self, qty: u64) -> Result<(), DomainError> {
        if qty == 0 {
            return Err(DomainError::NonPositiveQuantity);
        }
        self.available += qty;
        Ok(())
    }

    /// Removes `qty` units from the on-hand total, rejecting a zero quantity or a
    /// reservation larger than what is available.
    pub fn reserve(&mut self, qty: u64) -> Result<(), DomainError> {
        if qty == 0 {
            return Err(DomainError::NonPositiveQuantity);
        }
        if self.available < qty {
            return Err(DomainError::InsufficientStock {
                requested: qty,
                available: self.available,
            });
        }
        self.available -= qty;
        Ok(())
    }
}

/// A domain rule was violated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    NonPositiveQuantity,
    InsufficientStock { requested: u64, available: u64 },
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomainError::NonPositiveQuantity => write!(f, "quantity must be greater than zero"),
            DomainError::InsufficientStock {
                requested,
                available,
            } => write!(
                f,
                "insufficient stock: requested {requested}, only {available} available"
            ),
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

/// Outbound port: persistence of per-product stock, in the domain's language
/// (classic DDD — the repository interface lives in the domain, ADR 0003).
/// Implemented in `infrastructure`; the application adapts its [`RepositoryError`]
/// to the framework's `PortResult`. Each future is `Send` so a use case can
/// `.await` it inside a `UnitOfWork::transaction` on a multi-threaded runtime.
pub trait StockRepository {
    fn get_stock(
        &self,
        product_id: u128,
    ) -> impl Future<Output = Result<Option<StockItem>, RepositoryError>> + Send;

    /// Inserts or replaces the stock record for its product.
    fn save_stock(
        &self,
        stock: &StockItem,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
}

/// A shared [`StockRepository`] is itself a `StockRepository`: delegating through
/// the `Arc` lets the composition root hold ONE stock state behind an
/// `Arc<impl StockRepository>` and inject a clone into both the HTTP controller and
/// an event subscriber, so an `orders.placed` reservation and an
/// `available` query read and write the very same stock (ADR 0021, Fase 3). It
/// stays here, next to the port, because the port trait is local to this crate.
impl<R: StockRepository + Send + Sync> StockRepository for std::sync::Arc<R> {
    fn get_stock(
        &self,
        product_id: u128,
    ) -> impl Future<Output = Result<Option<StockItem>, RepositoryError>> + Send {
        (**self).get_stock(product_id)
    }

    fn save_stock(
        &self,
        stock: &StockItem,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send {
        (**self).save_stock(stock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receive_rejects_zero_quantity() {
        let mut s = StockItem::new(1);
        assert_eq!(s.receive(0), Err(DomainError::NonPositiveQuantity));
    }

    #[test]
    fn receive_then_reserve_tracks_the_total() {
        let mut s = StockItem::new(1);
        s.receive(10).unwrap();
        s.reserve(3).unwrap();
        assert_eq!(s.available, 7);
    }

    #[test]
    fn reserve_rejects_more_than_available() {
        let mut s = StockItem::new(1);
        s.receive(2).unwrap();
        assert_eq!(
            s.reserve(5),
            Err(DomainError::InsufficientStock {
                requested: 5,
                available: 2
            })
        );
        // The failed reservation left the stock untouched.
        assert_eq!(s.available, 2);
    }
}
