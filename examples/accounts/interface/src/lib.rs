//! Interface layer of the `accounts` module: the transport-facing shell.
//!
//! Depends on the `facade` trait only; it holds any `Accounts` implementation
//! and will expose it over HTTP/gRPC once the framework runtime is wired (a
//! later phase). It never sees `domain` — the application service already
//! mapped the domain entity to the secret-free DTO (ADR 0003).
//!
//! The composition that produces a ready `AccountsService` — the in-memory
//! infrastructure wired to the `seiva-testing` port doubles — is exercised in
//! this crate's own tests (via dev-dependencies), keeping production code
//! dependent on the facade alone.

#![forbid(unsafe_code)]

use accounts_facade::{Accounts, CreateUser, UserDto};

/// Wraps an `Accounts` implementation and exposes it. The concrete service
/// comes from `application`, chosen by the composition root in `apps/`.
pub struct AccountsApi<S: Accounts> {
    /// The wrapped service implementing the [`Accounts`] facade.
    pub service: S,
}

impl<S: Accounts> AccountsApi<S> {
    /// Wraps the given service.
    pub fn new(service: S) -> Self {
        Self { service }
    }

    /// The handler a `POST /api/v1/users` route will call.
    pub async fn create_user(&self, username: String, password: String) -> Result<UserDto, String> {
        self.service
            .create_user(CreateUser { username, password })
            .await
    }

    /// The handler a `GET /api/v1/users/{username}` route will call.
    pub async fn get_user(&self, username: String) -> Result<UserDto, String> {
        self.service.get_user(username).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accounts_application::AccountsService;
    use accounts_infrastructure::InMemoryCredentialRepository;
    use seiva_auth::verify_password;
    use seiva_testing::{InMemoryUnitOfWork, RecordingEventPublisher, SequenceIdGenerator};

    /// Builds a ready `AccountsApi` from the in-memory infrastructure wired to
    /// the `seiva-testing` port doubles — the composition the real `apps/`
    /// composition root performs, proven end to end here.
    fn composed_api() -> AccountsApi<
        AccountsService<
            InMemoryCredentialRepository,
            SequenceIdGenerator,
            InMemoryUnitOfWork,
            RecordingEventPublisher,
        >,
    > {
        AccountsApi::new(AccountsService {
            repository: InMemoryCredentialRepository::new(),
            ids: SequenceIdGenerator::new(),
            uow: InMemoryUnitOfWork::new(),
            events: RecordingEventPublisher::new(),
        })
    }

    #[tokio::test]
    async fn composed_service_creates_and_reads_a_user() {
        let api = composed_api();

        let created = api
            .create_user("alice".into(), "hunter2".into())
            .await
            .unwrap();
        assert_eq!(
            created,
            UserDto {
                id: 1,
                username: "alice".into()
            }
        );

        let fetched = api.get_user("alice".into()).await.unwrap();
        assert_eq!(fetched, created);

        // The persisted credential is a verifiable PHC hash, never the plaintext.
        let stored = api.service.repository.users();
        assert_eq!(stored.len(), 1);
        assert!(stored[0].password_hash.starts_with("$argon2id$"));
        assert!(verify_password("hunter2", &stored[0].password_hash).unwrap());

        // The committed event was recorded through the composed publisher.
        assert_eq!(api.service.events.topics(), ["accounts.user_created"]);
    }
}
