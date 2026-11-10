//! Domain layer of the `polls` module: the `Question` and `Choice` entities,
//! their rules AND the repository port.
//!
//! Depends on nothing but `shared/kernel` and std. The rules — a question and a
//! choice must each carry non-empty text — live here, and so does the REPOSITORY
//! PORT: classic DDD puts the repository interface in the domain (its ubiquitous
//! language), framed in a domain error, not the framework's (ADR 0003). The
//! application adapts it. This is the Seiva home of what Django expresses as the
//! `Question` and `Choice` models.

use std::future::Future;

/// A poll question. Construct it through [`Question::new`] so the rule is always
/// enforced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub id: u128,
    pub text: String,
}

impl Question {
    /// Creates a question, rejecting empty or whitespace-only text.
    pub fn new(id: u128, text: impl Into<String>) -> Result<Self, DomainError> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(DomainError::EmptyQuestion);
        }
        Ok(Self { id, text })
    }
}

/// A choice belonging to a question, with its running vote tally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub id: u128,
    pub question_id: u128,
    pub text: String,
    pub votes: u64,
}

impl Choice {
    /// Creates a choice with zero votes, rejecting empty or whitespace-only text.
    pub fn new(id: u128, question_id: u128, text: impl Into<String>) -> Result<Self, DomainError> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(DomainError::EmptyChoice);
        }
        Ok(Self {
            id,
            question_id,
            text,
            votes: 0,
        })
    }

    /// Records one vote for this choice (Django: `choice.votes += 1`).
    pub fn cast_vote(&mut self) {
        self.votes += 1;
    }
}

/// A domain rule was violated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    EmptyQuestion,
    EmptyChoice,
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomainError::EmptyQuestion => write!(f, "question text must not be empty"),
            DomainError::EmptyChoice => write!(f, "choice text must not be empty"),
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
pub trait PollRepository {
    fn save_question(
        &self,
        question: &Question,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    fn save_choice(
        &self,
        choice: &Choice,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    fn list_questions(&self)
        -> impl Future<Output = Result<Vec<Question>, RepositoryError>> + Send;

    fn get_question(
        &self,
        id: u128,
    ) -> impl Future<Output = Result<Option<Question>, RepositoryError>> + Send;

    fn choices_for(
        &self,
        question_id: u128,
    ) -> impl Future<Output = Result<Vec<Choice>, RepositoryError>> + Send;

    /// Records one vote for a choice and returns the updated choice, or `None`
    /// if no such choice exists.
    fn record_vote(
        &self,
        choice_id: u128,
    ) -> impl Future<Output = Result<Option<Choice>, RepositoryError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_question_text() {
        assert_eq!(Question::new(1, "   "), Err(DomainError::EmptyQuestion));
        assert!(Question::new(1, "What's new?").is_ok());
    }

    #[test]
    fn rejects_empty_choice_text() {
        assert_eq!(Choice::new(1, 1, ""), Err(DomainError::EmptyChoice));
        assert!(Choice::new(1, 1, "Not much").is_ok());
    }

    #[test]
    fn a_fresh_choice_starts_at_zero_and_counts_votes() {
        let mut c = Choice::new(1, 1, "The sky").unwrap();
        assert_eq!(c.votes, 0);
        c.cast_vote();
        c.cast_vote();
        assert_eq!(c.votes, 2);
    }
}
