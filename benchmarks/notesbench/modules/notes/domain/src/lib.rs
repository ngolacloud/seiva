//! Domain layer: entities, value objects and rules.
//!
//! Depends on nothing but `shared/kernel`. The rule — a record's title must
//! not be empty — lives here, not in the application or the database. Must
//! NOT depend on any other layer, the framework or DORM.

/// A record. Construct it through [`Record::new`] so the rule is enforced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The record's identifier.
    pub id: u128,
    /// The record's title.
    pub title: String,
}

impl Record {
    /// Creates a record, rejecting an empty or whitespace-only title.
    pub fn new(id: u128, title: impl Into<String>) -> Result<Self, DomainError> {
        let title = title.into();
        if title.trim().is_empty() {
            return Err(DomainError::EmptyTitle);
        }
        Ok(Self { id, title })
    }
}

/// A domain rule was violated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    /// The record's title was empty or whitespace only.
    EmptyTitle,
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomainError::EmptyTitle => write!(f, "record title must not be empty"),
        }
    }
}

impl std::error::Error for DomainError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_title() {
        assert_eq!(Record::new(1, "   "), Err(DomainError::EmptyTitle));
        assert!(Record::new(1, "hello").is_ok());
    }
}
