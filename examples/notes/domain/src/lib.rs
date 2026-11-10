//! Domain layer of the `notes` module: the `Note` entity, its rules AND the
//! repository port.
//!
//! Depends on nothing but `shared/kernel` and std. The rule — a note's title must
//! not be empty — lives here, and so does the REPOSITORY PORT: classic DDD puts
//! the repository interface in the domain (its ubiquitous language), framed in a
//! domain error, not the framework's (ADR 0003). The application adapts it.

use std::future::Future;

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
/// framework's `PortResult`. The future is `Send` so a use case can `.await` it
/// inside a `UnitOfWork::transaction` on a multi-threaded runtime.
pub trait NoteRepository {
    /// Persists a note.
    fn save(&self, note: &Note) -> impl Future<Output = Result<(), RepositoryError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_title() {
        assert_eq!(Note::new(1, "   "), Err(DomainError::EmptyTitle));
        assert!(Note::new(1, "hello").is_ok());
    }
}
