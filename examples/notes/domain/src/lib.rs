//! Domain layer of the `notes` module: the `Note` entity and its rules.
//!
//! Depends on nothing but `shared/kernel`. The rule — a note's title must not be
//! empty — lives here, not in the application or the database.

/// A note. Construct it through [`Note::new`] so the rule is always enforced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: u128,
    pub title: String,
}

impl Note {
    /// Creates a note, rejecting an empty or whitespace-only title.
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
    EmptyTitle,
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomainError::EmptyTitle => write!(f, "note title must not be empty"),
        }
    }
}

impl std::error::Error for DomainError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_title() {
        assert_eq!(Note::new(1, "   "), Err(DomainError::EmptyTitle));
        assert!(Note::new(1, "hello").is_ok());
    }
}
