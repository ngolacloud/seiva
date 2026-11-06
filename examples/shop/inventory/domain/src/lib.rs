//! Domain layer of the `inventory` module: the `StockItem` entity and its rules.
//!
//! Depends on nothing but `shared/kernel`. The rules — a received or reserved
//! quantity must be positive, and a reservation can never exceed what is on
//! hand — live here, not in the application or the database.

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
