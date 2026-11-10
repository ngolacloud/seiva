//! Domain layer of the `notifications` module: the `Notification` entity and its
//! rule.
//!
//! Depends on nothing but `shared/kernel`. The rule — a notification must have a
//! non-empty recipient — lives here, not in the application or the database. The
//! REPOSITORY PORT also lives here: classic DDD puts the repository interface in
//! the domain's ubiquitous language, framed in a domain error (ADR 0003).

use std::future::Future;

/// A message sent to a recipient. Construct it through [`Notification::new`] so
/// the rule is always enforced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub id: u128,
    pub to: String,
    pub subject: String,
    pub body: String,
}

impl Notification {
    /// Creates a notification, rejecting an empty or whitespace-only recipient.
    pub fn new(
        id: u128,
        to: impl Into<String>,
        subject: impl Into<String>,
        body: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let to = to.into();
        if to.trim().is_empty() {
            return Err(DomainError::EmptyRecipient);
        }
        Ok(Self {
            id,
            to,
            subject: subject.into(),
            body: body.into(),
        })
    }
}

/// A domain rule was violated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    EmptyRecipient,
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomainError::EmptyRecipient => {
                write!(f, "notification recipient must not be empty")
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

/// Outbound repository port, in the domain's language (classic DDD — the
/// repository interface lives in the domain, ADR 0003). Implemented in
/// `infrastructure`; the application adapts its [`RepositoryError`] to the
/// framework's `PortResult`. Each future is `Send` so a use case can `.await` it
/// inside a `UnitOfWork::transaction` on a multi-threaded runtime.
pub trait NotificationRepository {
    fn save_notification(
        &self,
        notification: &Notification,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    fn list_notifications(
        &self,
    ) -> impl Future<Output = Result<Vec<Notification>, RepositoryError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_an_empty_recipient() {
        assert_eq!(
            Notification::new(1, "   ", "Hi", "Body"),
            Err(DomainError::EmptyRecipient)
        );
    }

    #[test]
    fn accepts_a_valid_notification() {
        let n = Notification::new(1, "ada@example.com", "Order placed", "Thanks!").unwrap();
        assert_eq!(n.to, "ada@example.com");
        assert_eq!(n.subject, "Order placed");
    }
}
