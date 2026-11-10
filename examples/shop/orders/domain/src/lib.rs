//! Domain layer of the `orders` module: the `Order` entity, its `Line` value and
//! their rules.
//!
//! Depends on nothing but `shared/kernel`. The rule — an order must have at
//! least one line and every line a positive quantity — lives here, not in the
//! application or the database. The REPOSITORY PORT lives here too: classic DDD
//! puts the repository interface in the domain (its ubiquitous language), framed
//! in a domain error, not the framework's (ADR 0003). The application adapts it.

use std::future::Future;

/// One line of an order: a quantity of a single product.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub product_id: u128,
    pub qty: u64,
}

/// A customer order. Construct it through [`Order::new`] so the rule is always
/// enforced. A freshly placed order has status `"placed"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Order {
    pub id: u128,
    pub customer: String,
    pub lines: Vec<Line>,
    pub status: String,
}

impl Order {
    /// Creates an order, rejecting an order with no lines or any line whose
    /// quantity is zero.
    pub fn new(
        id: u128,
        customer: impl Into<String>,
        lines: Vec<Line>,
    ) -> Result<Self, DomainError> {
        if lines.is_empty() {
            return Err(DomainError::NoLines);
        }
        if lines.iter().any(|l| l.qty == 0) {
            return Err(DomainError::ZeroQuantityLine);
        }
        Ok(Self {
            id,
            customer: customer.into(),
            lines,
            status: "placed".to_string(),
        })
    }
}

/// A domain rule was violated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    NoLines,
    ZeroQuantityLine,
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomainError::NoLines => write!(f, "an order must have at least one line"),
            DomainError::ZeroQuantityLine => {
                write!(f, "every order line must have a quantity greater than zero")
            }
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

/// Outbound port: persistence of orders, in the domain's language (classic DDD —
/// the repository interface lives in the domain, ADR 0003). Implemented in
/// `infrastructure`; the application adapts its [`RepositoryError`] to the
/// framework's `PortResult`. Each future is `Send` so a use case can `.await` it
/// inside a `UnitOfWork::transaction` on a multi-threaded runtime.
pub trait OrderRepository {
    fn save_order(&self, order: &Order)
        -> impl Future<Output = Result<(), RepositoryError>> + Send;

    fn get_order(
        &self,
        id: u128,
    ) -> impl Future<Output = Result<Option<Order>, RepositoryError>> + Send;

    fn list_orders(&self) -> impl Future<Output = Result<Vec<Order>, RepositoryError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(product_id: u128, qty: u64) -> Line {
        Line { product_id, qty }
    }

    #[test]
    fn rejects_an_order_with_no_lines() {
        assert_eq!(Order::new(1, "Ada", vec![]), Err(DomainError::NoLines));
    }

    #[test]
    fn rejects_a_zero_quantity_line() {
        assert_eq!(
            Order::new(1, "Ada", vec![line(10, 0)]),
            Err(DomainError::ZeroQuantityLine)
        );
    }

    #[test]
    fn a_valid_order_starts_as_placed() {
        let o = Order::new(1, "Ada", vec![line(10, 2), line(11, 1)]).unwrap();
        assert_eq!(o.status, "placed");
        assert_eq!(o.lines.len(), 2);
    }
}
