//! Domain layer of the `notifications` module: the `Notification` entity and its
//! rule.
//!
//! Depends on nothing but `shared/kernel`. The rule — a notification must have a
//! non-empty recipient — lives here, not in the application or the database.

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
