//! Infrastructure layer of the `accounts` module: adapters that implement the
//! application's outbound ports.
//!
//! Depends on `application` + `domain` + `seiva-ports`. **DORM-free**: in a real
//! module the repository is DORM-backed (provided via `seiva-data-dorm` behind
//! the `dorm` feature); here it is an in-memory adapter, so the wiring is
//! provable without a database and the crate builds without the private DORM
//! repo.

#![forbid(unsafe_code)]

use accounts_domain::{CredentialRepository, RepositoryError, User};
use std::collections::HashMap;
use std::sync::Mutex;

/// An in-memory [`CredentialRepository`], keyed by username. Replace with the
/// DORM-backed adapter (`seiva-data-dorm`, `dorm` feature) in production.
#[derive(Default)]
pub struct InMemoryCredentialRepository {
    users: Mutex<HashMap<String, User>>,
}

impl InMemoryCredentialRepository {
    /// Creates an empty repository.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every user saved so far — for composition-level assertions.
    pub fn users(&self) -> Vec<User> {
        self.users
            .lock()
            .expect("repository poisoned")
            .values()
            .cloned()
            .collect()
    }
}

impl CredentialRepository for InMemoryCredentialRepository {
    async fn save(&self, user: &User) -> Result<(), RepositoryError> {
        self.users
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .insert(user.username.clone(), user.clone());
        Ok(())
    }

    async fn find_by_username(&self, username: &str) -> Result<Option<User>, RepositoryError> {
        Ok(self
            .users
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .get(username)
            .cloned())
    }
}
