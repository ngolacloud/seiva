//! Domain layer of the `accounts` module: the `User` entity and its rules.
//!
//! Depends on nothing but `shared/kernel`. The rules — a non-empty, bounded
//! username and a non-empty stored credential — live here, not in the
//! application or the database.
//!
//! The domain never sees a plaintext password. Hashing is the application
//! layer's concern (it uses `seiva-auth`); [`User::new`] accepts an
//! **already-hashed** credential (a PHC string) and only checks that it is
//! present. This keeps the domain free of any cryptography dependency.

#![forbid(unsafe_code)]

use std::future::Future;

/// The maximum length, in bytes, of a username. A small, explicit bound so a
/// pathological input cannot be stored.
pub const MAX_USERNAME_LEN: usize = 64;

/// A user account. Construct it through [`User::new`] so the rules are always
/// enforced.
///
/// The `password_hash` is a PHC string produced by the application layer (for
/// example `$argon2id$v=19$...`). The domain treats it as an opaque, required
/// credential: it is never the plaintext password, and it is never exposed
/// across the module's facade (the response DTO has no hash field).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// The account's stable identifier.
    pub id: u128,
    /// The account's username (validated non-empty and bounded).
    pub username: String,
    /// The stored Argon2id PHC credential string. Never the plaintext.
    pub password_hash: String,
}

impl User {
    /// Creates a user, enforcing the domain rules.
    ///
    /// The `username` is rejected when empty/whitespace-only or longer than
    /// [`MAX_USERNAME_LEN`] bytes. The `password_hash` must already be a hashed
    /// credential and is rejected when empty — the domain refuses to construct
    /// an account with no credential, but does not (and cannot) hash anything
    /// itself.
    ///
    /// # Errors
    ///
    /// Returns the matching [`DomainError`] variant when a rule is violated.
    pub fn new(
        id: u128,
        username: impl Into<String>,
        password_hash: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let username = username.into();
        let trimmed = username.trim();
        if trimmed.is_empty() {
            return Err(DomainError::EmptyUsername);
        }
        if username.len() > MAX_USERNAME_LEN {
            return Err(DomainError::UsernameTooLong {
                max: MAX_USERNAME_LEN,
                actual: username.len(),
            });
        }
        let password_hash = password_hash.into();
        if password_hash.is_empty() {
            return Err(DomainError::EmptyCredential);
        }
        Ok(Self {
            id,
            username,
            password_hash,
        })
    }
}

/// A domain rule was violated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    /// The username was empty or whitespace-only.
    EmptyUsername,
    /// The username exceeded [`MAX_USERNAME_LEN`] bytes.
    UsernameTooLong {
        /// The permitted maximum length, in bytes.
        max: usize,
        /// The rejected username's actual length, in bytes.
        actual: usize,
    },
    /// The stored credential (PHC hash) was empty.
    EmptyCredential,
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomainError::EmptyUsername => write!(f, "username must not be empty"),
            DomainError::UsernameTooLong { max, actual } => {
                write!(
                    f,
                    "username is {actual} bytes, exceeding the maximum of {max}"
                )
            }
            DomainError::EmptyCredential => write!(f, "credential hash must not be empty"),
        }
    }
}

impl std::error::Error for DomainError {}

/// A repository operation failed. A DOMAIN-level error: the repository is part of the domain's language (ADR 0003).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryError(pub String);
impl std::fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for RepositoryError {}

/// Outbound port: persistence of user credentials, in the domain's language. In
/// classic DDD the repository interface lives in the DOMAIN (its ubiquitous
/// language, ADR 0003); `infrastructure` implements it and the application
/// adapts its [`RepositoryError`] to the framework's `PortResult`. The futures
/// are `Send` so a use case can `.await` them inside a `UnitOfWork::transaction`
/// on a multi-threaded runtime.
pub trait CredentialRepository {
    /// Persists a user. The stored `password_hash` is a PHC string; the
    /// plaintext password never reaches this port.
    fn save(&self, user: &User) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Finds a user by exact username, or `None` when none exists.
    fn find_by_username(
        &self,
        username: &str,
    ) -> impl Future<Output = Result<Option<User>, RepositoryError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_username() {
        assert_eq!(
            User::new(1, "   ", "$argon2id$hash"),
            Err(DomainError::EmptyUsername)
        );
    }

    #[test]
    fn rejects_overlong_username() {
        let long = "a".repeat(MAX_USERNAME_LEN + 1);
        assert_eq!(
            User::new(1, long, "$argon2id$hash"),
            Err(DomainError::UsernameTooLong {
                max: MAX_USERNAME_LEN,
                actual: MAX_USERNAME_LEN + 1,
            })
        );
    }

    #[test]
    fn rejects_empty_credential() {
        assert_eq!(User::new(1, "alice", ""), Err(DomainError::EmptyCredential));
    }

    #[test]
    fn accepts_a_valid_user() {
        let user = User::new(1, "alice", "$argon2id$hash").unwrap();
        assert_eq!(user.id, 1);
        assert_eq!(user.username, "alice");
        assert_eq!(user.password_hash, "$argon2id$hash");
    }
}
