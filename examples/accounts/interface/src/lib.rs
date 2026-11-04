//! Interface layer of the `accounts` module: the HTTP transport.
//!
//! LAYERING DECISION: the `#[seiva::controller]` lives HERE, not in
//! `application`, so the application layer stays transport-free (it depends only
//! on `domain`, `facade` and `seiva-ports`, like `examples/notes`). The
//! controller is applied to the CONCRETE [`AccountsController`] adapter, which
//! delegates to the module's default in-memory service wired here. A concrete
//! self type is required: `#[seiva::controller]` dispatches through the facade
//! trait, whose `async fn` methods are only known to return `Send` futures (as
//! axum's router demands) for a concrete type — a generic service would leave the
//! trait's return-position future unbounded.
//!
//! The macro generates an `accounts_router(service)` builder whose route version
//! and per-route permission come from the `AccountsFacade` descriptor at run
//! time. [`router`] composes the default service and returns that router, ready
//! to merge into a server application.
//!
//! # DORM-free
//!
//! The default service is in-memory: `AccountsService` over
//! `InMemoryCredentialRepository` and the in-process ports. No `dorm-*` crate is linked; a
//! DORM-backed repository and the transactional ports arrive through the
//! framework's `dorm` feature later.
//!
//! # Generated routes (first cut)
//!
//! The controller keys the HTTP method and path off the facade **method name**.
//! `create_user` and `get_user` are neither `create` nor `get`, so both fall to
//! the generic write branch — a `POST` on `/{version}/{resource}/{method}`:
//!
//! | facade method | route                        | success | required permission |
//! |---------------|------------------------------|---------|---------------------|
//! | `create_user` | `POST /v1/accounts/create_user` | `200` | `accounts.create` |
//! | `get_user`    | `POST /v1/accounts/get_user`    | `200` | `accounts.read`   |
//!
//! Both carry a JSON body validated by the controller's `Valid<T>` extractor
//! ([`CreateUser`] and [`GetUser`]). This is the shape the macro produces for
//! these method names; nicer REST paths (`GET /v1/accounts/{username}`) would
//! require renaming the facade methods to `create`/`get`, which is out of scope
//! here. The macro is not modified.
//!
//! The generated handlers return `Result<_, seiva_http::Problem>`, a large
//! value, so the large-`Err` lint is allowed crate-wide (as `seiva-http` itself
//! does).

#![forbid(unsafe_code)]
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use delonix_seiva::controller;

use accounts_application::AccountsService;
use accounts_facade::{Accounts, AccountsFacade, CreateUser, GetUser, UserDto, UserError};
use accounts_infrastructure::InMemoryCredentialRepository;

use runtime::{DirectUnitOfWork, NoopEventPublisher, SystemIds};

/// In-process ports that let the interface compose a runnable `accounts` service
/// without the DORM data layer. Mirrors `seiva-cli`'s `app_ports`; de-duplicating
/// the two behind a shared crate is a follow-up.
mod runtime {
    use async_trait::async_trait;
    use seiva_ports::{EventPublisher, Id, IdGenerator, PortResult, UnitOfWork};
    use std::future::Future;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A time-seeded [`IdGenerator`]: the 16 id bytes are the current time in
    /// nanoseconds, big-endian. Unique enough for the default in-memory service;
    /// the DORM gateway supplies a UUID generator for the persistent path.
    pub struct SystemIds;

    impl IdGenerator for SystemIds {
        fn new_id(&self) -> Id {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            Id(nanos.to_be_bytes())
        }
    }

    /// A pass-through [`UnitOfWork`]: it runs the use-case body directly, with no
    /// real transaction. The DORM gateway's unit of work wraps the body in a real
    /// database transaction and the transactional outbox.
    pub struct DirectUnitOfWork;

    impl UnitOfWork for DirectUnitOfWork {
        async fn transaction<T, F, Fut>(&self, work: F) -> PortResult<T>
        where
            F: FnOnce() -> Fut + Send,
            Fut: Future<Output = PortResult<T>> + Send,
            T: Send,
        {
            work().await
        }
    }

    /// A no-op [`EventPublisher`]: it discards the emitted event. The DORM gateway
    /// writes it to the transactional outbox instead. `#[async_trait]` because the
    /// port trait is object-safe and declared that way.
    pub struct NoopEventPublisher;

    #[async_trait]
    impl EventPublisher for NoopEventPublisher {
        async fn publish(&self, _topic: &str, _payload: &[u8]) -> PortResult<()> {
            Ok(())
        }
    }
}

/// The module's default, in-memory `Accounts` service: the application service
/// over the in-memory repository and the in-process `runtime` ports. Swap
/// these ports (e.g. a DORM-backed repository) in a production composition root.
pub type DefaultService =
    AccountsService<InMemoryCredentialRepository, SystemIds, DirectUnitOfWork, NoopEventPublisher>;

/// Transport adapter: a concrete `Accounts` service exposed over HTTP.
///
/// It holds the default service behind an [`Arc`] so it is `Clone + Send + Sync +
/// 'static` — the bound the generated `accounts_router` places on its axum state —
/// without requiring the service or its ports to be `Clone`.
#[derive(Clone)]
pub struct AccountsController(Arc<DefaultService>);

impl AccountsController {
    /// Builds the controller over the default in-memory service.
    pub fn new() -> Self {
        AccountsController(Arc::new(AccountsService {
            repository: InMemoryCredentialRepository::new(),
            ids: SystemIds,
            uow: DirectUnitOfWork,
            events: NoopEventPublisher,
        }))
    }
}

impl Default for AccountsController {
    fn default() -> Self {
        Self::new()
    }
}

impl AccountsController {
    /// Runs a use case on the blocking pool and awaits its result.
    ///
    /// The `accounts` use case drives `seiva_ports::UnitOfWork::transaction`,
    /// whose generic `F: FnOnce() -> Fut` closure under an `async fn`-in-trait
    /// return type produces a future rustc cannot prove `Send` in a generic async
    /// context (a known higher-ranked-lifetime limitation). axum requires a
    /// `Send` handler future, so the use-case future is confined to a
    /// [`tokio::task::spawn_blocking`] thread with its own current-thread runtime;
    /// the future this method returns only awaits the `JoinHandle`, which IS
    /// `Send`. This also keeps the CPU-bound Argon2id hashing off the async worker
    /// threads — the right home for it. The proper fix (a `Send`-friendly
    /// `UnitOfWork`) lives in `seiva-ports` and is out of scope here.
    async fn run<T, Fut>(
        &self,
        use_case: impl FnOnce(Arc<DefaultService>) -> Fut + Send + 'static,
    ) -> Result<T, UserError>
    where
        Fut: std::future::Future<Output = Result<T, UserError>>,
        T: Send + 'static,
    {
        let service = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("building a current-thread runtime for the accounts use case");
            runtime.block_on(use_case(service))
        })
        .await
        .unwrap_or_else(|join_error| {
            Err(UserError(format!(
                "accounts use case panicked: {join_error}"
            )))
        })
    }
}

#[controller]
impl Accounts for AccountsController {
    async fn create_user(&self, cmd: CreateUser) -> Result<UserDto, UserError> {
        self.run(move |service| async move { service.create_user(cmd).await })
            .await
    }

    async fn get_user(&self, query: GetUser) -> Result<UserDto, UserError> {
        self.run(move |service| async move { service.get_user(query).await })
            .await
    }
}

/// Builds the `accounts` module's HTTP router, wired to the default in-memory
/// service. Merge it into a server application in the composition root; a request
/// to an accounts route then hits the real module use case (validation, Argon2id
/// hashing and the `accounts.user_created` event).
pub fn router() -> axum::Router {
    accounts_router(AccountsController::new())
}

/// Wraps an `Accounts` implementation and exposes a plain-Rust handler surface.
///
/// Retained for the CLI (`seiva user create`) and the composition test, which
/// drive the use case without the HTTP transport. It maps the facade's
/// [`UserError`] to a plain `String` so its callers keep the simple error shape
/// they had before the facade gained a typed wire error.
pub struct AccountsApi<S: Accounts> {
    /// The wrapped service implementing the [`Accounts`] facade.
    pub service: S,
}

impl<S: Accounts> AccountsApi<S> {
    /// Wraps the given service.
    pub fn new(service: S) -> Self {
        Self { service }
    }

    /// Creates a user, returning the public [`UserDto`] or the module's error
    /// message.
    pub async fn create_user(&self, username: String, password: String) -> Result<UserDto, String> {
        self.service
            .create_user(CreateUser { username, password })
            .await
            .map_err(|err| err.0)
    }

    /// Looks a user up by username, returning the public [`UserDto`] or the
    /// module's error message.
    pub async fn get_user(&self, username: String) -> Result<UserDto, String> {
        self.service
            .get_user(GetUser { username })
            .await
            .map_err(|err| err.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accounts_application::AccountsService;
    use accounts_infrastructure::InMemoryCredentialRepository;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use seiva_auth::verify_password;
    use seiva_http::{Principal, PROBLEM_JSON};
    use seiva_testing::{InMemoryUnitOfWork, RecordingEventPublisher, SequenceIdGenerator};
    use serde_json::Value;
    use tower::ServiceExt;

    /// Reads a response body into a JSON value.
    async fn json_body(body: Body) -> Value {
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    // --- The composed in-memory service, driven as plain Rust via AccountsApi --

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

    // --- The generated controller router, driven via `oneshot` (no live port) --

    #[tokio::test]
    async fn create_route_is_forbidden_without_permission() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/accounts/create_user")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"username":"alice","password":"hunter2"}"#))
            .unwrap();

        let response = router().oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.headers()["content-type"], PROBLEM_JSON);
        let body = json_body(response.into_body()).await;
        assert_eq!(body["status"], 403);
        assert!(body["detail"].as_str().unwrap().contains("accounts.create"));
    }

    #[tokio::test]
    async fn create_route_succeeds_with_permission_and_hides_the_secret() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/accounts/create_user")
            .header("content-type", "application/json")
            .extension(Principal::new("user", ["accounts.create"]))
            .body(Body::from(r#"{"username":"alice","password":"hunter2"}"#))
            .unwrap();

        let response = router().oneshot(request).await.unwrap();

        // `create_user` is not the exact name `create`, so it is the generic
        // write branch: `200 OK`, not `201 Created`.
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response.into_body()).await;
        assert_eq!(body["username"], "alice");
        assert!(body.get("id").is_some(), "the created user carries an id");
        // The response DTO cannot carry a secret: prove none leaked.
        assert!(body.get("password").is_none());
        assert!(body.get("password_hash").is_none());
        let rendered = body.to_string();
        assert!(
            !rendered.contains("hunter2") && !rendered.contains("$argon2id$"),
            "no secret must appear in the response: {rendered}"
        );
    }

    #[tokio::test]
    async fn create_route_invalid_body_is_422_problem() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/accounts/create_user")
            .header("content-type", "application/json")
            .extension(Principal::new("user", ["accounts.create"]))
            .body(Body::from(r#"{"username":"   ","password":"hunter2"}"#))
            .unwrap();

        let response = router().oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.headers()["content-type"], PROBLEM_JSON);
        let body = json_body(response.into_body()).await;
        assert_eq!(body["status"], 422);
        assert!(body["detail"].as_str().unwrap().contains("username"));
    }

    #[tokio::test]
    async fn create_then_get_round_trips_through_the_real_use_case() {
        // One router instance so both requests share the in-memory repository.
        let app = accounts_router(AccountsController::new());

        let create = Request::builder()
            .method("POST")
            .uri("/v1/accounts/create_user")
            .header("content-type", "application/json")
            .extension(Principal::new("user", ["accounts.create"]))
            .body(Body::from(r#"{"username":"alice","password":"hunter2"}"#))
            .unwrap();
        let response = app.clone().oneshot(create).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let get = Request::builder()
            .method("POST")
            .uri("/v1/accounts/get_user")
            .header("content-type", "application/json")
            .extension(Principal::new("user", ["accounts.read"]))
            .body(Body::from(r#"{"username":"alice"}"#))
            .unwrap();
        let response = app.oneshot(get).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response.into_body()).await;
        assert_eq!(body["username"], "alice");
    }
}
