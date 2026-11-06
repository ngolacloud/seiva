//! Domain layer of the `catalog` module: the `Product` entity and its rules.
//!
//! Depends on nothing but `shared/kernel`. The rule — a product must carry a
//! non-empty name and a price above zero — lives here, not in the application
//! or the database.

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
