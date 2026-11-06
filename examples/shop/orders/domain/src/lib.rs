//! Domain layer of the `orders` module: the `Order` entity, its `Line` value and
//! their rules.
//!
//! Depends on nothing but `shared/kernel`. The rule — an order must have at
//! least one line and every line a positive quantity — lives here, not in the
//! application or the database.

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
