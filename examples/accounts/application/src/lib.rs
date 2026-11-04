//! Application layer of the `accounts` module: the `AccountsService` (which
//! implements the public facade), the `CredentialRepository` outbound port, and
//! the create/get use cases.
//!
//! Depends on `domain`, its own `facade` (DTOs + trait), `seiva-ports` and
//! `seiva-auth`. It names no `delonix_seiva::` item — the framework coupling is
//! the facade's concern (ADR 0003). The service consumes the framework's
//! cross-cutting ports (`UnitOfWork`, `IdGenerator`, `EventPublisher`) through
//! `seiva-ports`, and hashes the plaintext password with `seiva-auth`
//! (Argon2id) so the domain and the repository only ever see the PHC hash.

#![forbid(unsafe_code)]
#![allow(async_fn_in_trait)]

use accounts_domain::User;
use accounts_facade::{Accounts, CreateUser, GetUser, UserDto, UserError};
use seiva_auth::hash_password;
use seiva_ports::{BoxError, EventPublisher, IdGenerator, PortResult, UnitOfWork};

/// An application-level failure that is not a domain-rule violation.
///
/// Kept small (unit variants, no owned payload) so a `Result<_, AccountsError>`
/// stays well under clippy's `result_large_err` threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountsError {
    /// A user with the requested username already exists.
    UsernameTaken,
    /// No user exists with the requested username.
    NotFound,
}

impl std::fmt::Display for AccountsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccountsError::UsernameTaken => write!(f, "username already taken"),
            AccountsError::NotFound => write!(f, "user not found"),
        }
    }
}

impl std::error::Error for AccountsError {}

/// Outbound port: persistence of user credentials, in the domain's language.
/// Implemented in `infrastructure` (over DORM or an external library; the
/// example ships an in-memory adapter). The futures are `Send` so a use case
/// can `.await` them inside a [`UnitOfWork::transaction`], which runs on a
/// multi-threaded runtime.
pub trait CredentialRepository {
    /// Persists a user. The stored `password_hash` is a PHC string; the
    /// plaintext password never reaches this port.
    fn save(&self, user: &User) -> impl std::future::Future<Output = PortResult<()>> + Send;

    /// Finds a user by exact username, or `None` when none exists.
    fn find_by_username(
        &self,
        username: &str,
    ) -> impl std::future::Future<Output = PortResult<Option<User>>> + Send;
}

/// The application service. It implements the [`Accounts`] facade by running the
/// use cases and mapping the domain entity to the secret-free DTO.
pub struct AccountsService<R, G, U, E> {
    /// The credential repository (outbound port).
    pub repository: R,
    /// The identifier generator.
    pub ids: G,
    /// The transaction boundary.
    pub uow: U,
    /// The domain-event publisher.
    pub events: E,
}

impl<R, G, U, E> AccountsService<R, G, U, E>
where
    R: CredentialRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    async fn create_user_uc(&self, cmd: CreateUser) -> PortResult<UserDto> {
        // Hash the plaintext BEFORE building the entity: the domain and the
        // repository only ever see the Argon2id PHC string, never the password.
        let password_hash = hash_password(&cmd.password).map_err(BoxError::from)?;

        // Validate in the domain (non-empty/bounded username, present
        // credential) before opening a transaction.
        let id = u128::from_be_bytes(self.ids.new_id().0);
        let user = User::new(id, cmd.username, password_hash).map_err(BoxError::from)?;

        // One transaction per use case (scope-based UnitOfWork, ADR 0004): the
        // duplicate check, the save AND the event publish run inside it, so with
        // the DORM gateway the event lands in the transactional outbox in the
        // SAME transaction as the business change — they commit together or not
        // at all. On a duplicate the body returns `Err`, so nothing is saved and
        // the transaction rolls back. The id bytes stand in for a serialized
        // event envelope.
        self.uow
            .transaction(|| async {
                if self
                    .repository
                    .find_by_username(&user.username)
                    .await?
                    .is_some()
                {
                    return Err(BoxError::from(AccountsError::UsernameTaken));
                }
                self.repository.save(&user).await?;
                self.events
                    .publish("accounts.user_created", &user.id.to_be_bytes())
                    .await
            })
            .await?;

        // The DTO carries no secret: only the id and the username cross the
        // boundary, never `user.password_hash`.
        Ok(UserDto {
            id: user.id,
            username: user.username,
        })
    }

    async fn get_user_uc(&self, username: String) -> PortResult<UserDto> {
        let user = self
            .repository
            .find_by_username(&username)
            .await?
            .ok_or_else(|| BoxError::from(AccountsError::NotFound))?;
        Ok(UserDto {
            id: user.id,
            username: user.username,
        })
    }
}

impl<R, G, U, E> Accounts for AccountsService<R, G, U, E>
where
    R: CredentialRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    async fn create_user(&self, cmd: CreateUser) -> Result<UserDto, UserError> {
        // Map the internal error to the contract's error type: no internal
        // detail leaks across the module boundary beyond a message.
        self.create_user_uc(cmd)
            .await
            .map_err(|err| UserError(err.to_string()))
    }

    async fn get_user(&self, query: GetUser) -> Result<UserDto, UserError> {
        self.get_user_uc(query.username)
            .await
            .map_err(|err| UserError(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seiva_auth::verify_password;
    use seiva_testing::{
        InMemoryUnitOfWork, RecordingEventPublisher, SequenceIdGenerator, TxOutcome,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRepo {
        users: Mutex<HashMap<String, User>>,
    }

    impl FakeRepo {
        fn get(&self, username: &str) -> Option<User> {
            self.users.lock().unwrap().get(username).cloned()
        }

        fn len(&self) -> usize {
            self.users.lock().unwrap().len()
        }
    }

    impl CredentialRepository for FakeRepo {
        async fn save(&self, user: &User) -> PortResult<()> {
            self.users
                .lock()
                .unwrap()
                .insert(user.username.clone(), user.clone());
            Ok(())
        }

        async fn find_by_username(&self, username: &str) -> PortResult<Option<User>> {
            Ok(self.users.lock().unwrap().get(username).cloned())
        }
    }

    fn service(
    ) -> AccountsService<FakeRepo, SequenceIdGenerator, InMemoryUnitOfWork, RecordingEventPublisher>
    {
        AccountsService {
            repository: FakeRepo::default(),
            ids: SequenceIdGenerator::new(),
            uow: InMemoryUnitOfWork::new(),
            events: RecordingEventPublisher::new(),
        }
    }

    fn cmd(username: &str, password: &str) -> CreateUser {
        CreateUser {
            username: username.into(),
            password: password.into(),
        }
    }

    #[tokio::test]
    async fn create_persists_commits_and_emits() {
        let svc = service();
        let dto = svc.create_user(cmd("alice", "hunter2")).await.unwrap();

        assert_eq!(dto.id, 1);
        assert_eq!(dto.username, "alice");
        assert_eq!(svc.repository.len(), 1);
        assert_eq!(svc.uow.outcomes(), [TxOutcome::Committed]);
        assert_eq!(svc.events.topics(), ["accounts.user_created"]);
    }

    #[tokio::test]
    async fn stored_credential_is_a_phc_hash_never_the_plaintext() {
        let svc = service();
        svc.create_user(cmd("alice", "hunter2")).await.unwrap();

        let stored = svc.repository.get("alice").expect("user was saved");
        // The plaintext was never persisted: the stored credential is an
        // Argon2id PHC string, not the password itself.
        assert_ne!(stored.password_hash, "hunter2");
        assert!(
            stored.password_hash.starts_with("$argon2id$"),
            "expected a PHC argon2id string, got: {}",
            stored.password_hash
        );
        // The stored hash verifies the right password and rejects a wrong one.
        assert!(verify_password("hunter2", &stored.password_hash).unwrap());
        assert!(!verify_password("wrong-password", &stored.password_hash).unwrap());
    }

    #[tokio::test]
    async fn dto_carries_no_secret() {
        let svc = service();
        let dto = svc.create_user(cmd("alice", "hunter2")).await.unwrap();

        // The DTO structurally cannot hold a secret (no password/hash field).
        // Prove the returned value equals the public projection exactly, and
        // that the plaintext does not appear in any of its fields.
        assert_eq!(
            dto,
            UserDto {
                id: 1,
                username: "alice".into(),
            }
        );
        assert_ne!(dto.username, "hunter2");
    }

    #[tokio::test]
    async fn duplicate_username_is_rejected() {
        let svc = service();
        svc.create_user(cmd("alice", "hunter2")).await.unwrap();

        let err = svc.create_user(cmd("alice", "different")).await;
        assert!(err.is_err());
        assert_eq!(err.unwrap_err().0, "username already taken");

        // Only the first user was saved; the duplicate rolled back.
        assert_eq!(svc.repository.len(), 1);
        assert_eq!(
            svc.uow.outcomes(),
            [TxOutcome::Committed, TxOutcome::RolledBack]
        );
        // Exactly one creation event was emitted (none for the rejected dup).
        assert_eq!(svc.events.topics(), ["accounts.user_created"]);
    }

    #[tokio::test]
    async fn invalid_username_is_rejected_by_the_domain_rule() {
        let svc = service();
        let err = svc.create_user(cmd("   ", "hunter2")).await;
        assert!(err.is_err());
        // Nothing was committed or emitted when the rule rejected the command.
        assert_eq!(svc.repository.len(), 0);
        assert!(svc.uow.outcomes().is_empty());
        assert_eq!(svc.events.topics(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn get_user_returns_the_dto_or_not_found() {
        let svc = service();
        svc.create_user(cmd("alice", "hunter2")).await.unwrap();

        let dto = svc
            .get_user(GetUser {
                username: "alice".into(),
            })
            .await
            .unwrap();
        assert_eq!(dto.username, "alice");

        let missing = svc
            .get_user(GetUser {
                username: "nobody".into(),
            })
            .await;
        assert_eq!(missing.unwrap_err().0, "user not found");
    }
}
