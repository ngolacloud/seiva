//! Domain layer of the `payments` module: the `Payment` entity and its rules.
//!
//! Depends on nothing but `shared/kernel`. The *outcome* of a charge (whether an
//! amount is captured or declined) is an application-level policy and lives in
//! the use case; the domain enforces the invariant that a **captured** payment
//! can never be for a zero amount.
//!
//! Classic DDD also puts the REPOSITORY PORT here: the repository interface is
//! part of the domain's ubiquitous language, framed in a domain error, not the
//! framework's (ADR 0003). The application adapts it.

use std::future::Future;

/// A recorded payment against an order. Construct it through
/// [`Payment::captured`] or [`Payment::declined`]; both set the `status` so an
/// invalid status can never exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payment {
    pub id: u128,
    pub order_id: u128,
    pub amount_cents: u64,
    pub status: String,
}

impl Payment {
    /// Records a captured payment, rejecting a zero amount — a captured charge
    /// must move money.
    pub fn captured(id: u128, order_id: u128, amount_cents: u64) -> Result<Self, DomainError> {
        if amount_cents == 0 {
            return Err(DomainError::ZeroAmountCapture);
        }
        Ok(Self {
            id,
            order_id,
            amount_cents,
            status: "captured".to_string(),
        })
    }

    /// Records a declined payment for the attempted amount.
    pub fn declined(id: u128, order_id: u128, amount_cents: u64) -> Self {
        Self {
            id,
            order_id,
            amount_cents,
            status: "declined".to_string(),
        }
    }

    /// Whether this payment was captured.
    pub fn is_captured(&self) -> bool {
        self.status == "captured"
    }
}

/// A domain rule was violated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    ZeroAmountCapture,
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomainError::ZeroAmountCapture => {
                write!(
                    f,
                    "a captured payment must be for an amount greater than zero"
                )
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

/// Outbound port: persistence of payments, in the domain's language (classic DDD
/// — the repository interface lives in the domain, ADR 0003). Implemented in
/// `infrastructure`; the application adapts its [`RepositoryError`] to the
/// framework's `PortResult`. Each future is `Send` so a use case can `.await` it
/// inside a `UnitOfWork::transaction` on a multi-threaded runtime.
pub trait PaymentRepository {
    fn save_payment(
        &self,
        payment: &Payment,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    fn get_payment(
        &self,
        id: u128,
    ) -> impl Future<Output = Result<Option<Payment>, RepositoryError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_captured_payment_cannot_be_zero() {
        assert_eq!(
            Payment::captured(1, 7, 0),
            Err(DomainError::ZeroAmountCapture)
        );
    }

    #[test]
    fn captured_sets_the_status() {
        let p = Payment::captured(1, 7, 2500).unwrap();
        assert!(p.is_captured());
        assert_eq!(p.amount_cents, 2500);
    }

    #[test]
    fn declined_sets_the_status() {
        let p = Payment::declined(1, 7, 0);
        assert!(!p.is_captured());
        assert_eq!(p.status, "declined");
    }
}
