//! Interface layer of the `accounts` module: the HTTP transport.
//!
//! LAYERING DECISION: the `#[seiva::controller]` lives HERE, not in
//! `application`, so the application layer stays transport-free (it depends only
//! on `domain`, `facade` and `seiva-ports`, like `examples/notes`). The
//! controller [`AccountsController<R>`] is GENERIC over the repository port `R`,
//! so this layer depends only on `application` (for the service and the
//! `CredentialRepository` port) and never on the `infrastructure` crate — the
//! layering rule forbids `interface -> infrastructure`. A concrete self type is
//! still what `#[seiva::controller]` dispatches through (the trait's `async fn`
//! methods are only known to return `Send` futures for a concrete type), but
//! `AccountsController<R>` is concrete once `R` is chosen, and the macro
//! propagates the generic + its `where` bound onto the generated router.
//!
//! The macro generates an `accounts_router(service)` builder whose route version
//! and per-route permission come from the `AccountsFacade` descriptor at run
//! time. The COMPOSITION ROOT (e.g. `seiva-server`) picks the concrete repository
//! — `AccountsController::new(InMemoryCredentialRepository::new())` — and merges
//! `accounts_router(controller)` into its application.
//!
//! # DORM-free
//!
//! The in-process ports (`SystemIds`, `DirectUnitOfWork`, `NoopEventPublisher`)
//! are defined inline; the repository is injected by the composition root. No
//! `dorm-*` crate is linked; a DORM-backed repository and the transactional ports
//! arrive through the framework's `dorm` feature later.
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

#![forbid(unsafe_code)]

use std::sync::Arc;

use delonix_seiva::admin::{AdminPermissions, AdminResource};
use delonix_seiva::{controller, register_admin};

use accounts_application::{AccountsService, CredentialRepository};
use accounts_facade::{Accounts, AccountsFacade, CreateUser, GetUser, UserDto, UserError};
use seiva_http::Ctx;
use seiva_ports::{EventPublisher, IdGenerator, Scope, UnitOfWork};

// Make `accounts` visible to the optional admin console (ADR 0009). This couples
// the module only to the framework contract (`delonix-seiva`), NOT to the admin
// UI crate: with the console unmounted this is inert link-time data. A module
// that omits this registration stays private to the console. The resource id
// matches the module's route segment (`/v1/accounts/...`) so the console can
// pair it with the operations in `/openapi.json`, and each permission matches
// the facade's `#[requires(..)]` so the server-side guard stays the authority.
register_admin!(AdminResource {
    namespace: "accounts",
    resource: "accounts",
    label: "Accounts",
    facade: "Accounts",
    version: "v1",
    list_fields: &["id", "username"],
    search_fields: &["username"],
    filters: &[],
    ordering: &["username"],
    page_size: 25,
    permissions: AdminPermissions {
        view: "accounts.read",
        create: "accounts.create",
        change: "",
        delete: "",
    },
});

use runtime::{DirectUnitOfWork, NoopEventPublisher, SystemIds};

/// In-process ports that let the interface compose a runnable `accounts` service
/// without the DORM data layer. Mirrors `seiva-cli`'s `app_ports`; de-duplicating
/// the two behind a shared crate is a follow-up.
mod runtime {
    use async_trait::async_trait;
    use seiva_ports::{EventPublisher, Id, IdGenerator, PortResult, Scope, UnitOfWork};
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
        fn transaction<'a, T, F, Fut>(
            &'a self,
            _scope: Scope,
            work: F,
        ) -> std::pin::Pin<Box<dyn Future<Output = PortResult<T>> + Send + 'a>>
        where
            F: FnOnce() -> Fut + Send + 'a,
            Fut: Future<Output = PortResult<T>> + Send + 'a,
            T: Send + 'a,
        {
            Box::pin(async move { work().await })
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

/// The concrete `Accounts` service this controller wraps: the application
/// service over a caller-supplied repository `R` and the in-process `runtime`
/// ports. Named here WITHOUT the infrastructure crate, so the interface layer
/// depends only on `application` — the layering rule forbids
/// `interface -> infrastructure`. The composition root supplies the concrete
/// repository (in-memory, or a DORM-backed one).
type Service<R> = AccountsService<R, SystemIds, DirectUnitOfWork, NoopEventPublisher>;

/// Transport adapter: a concrete `Accounts` service exposed over HTTP, generic
/// over the repository port `R`.
///
/// It holds the service behind an [`Arc`] so it is `Clone + Send + Sync +
/// 'static` — the bound the generated `accounts_router` places on its axum state —
/// without requiring the service or its ports to be `Clone`.
pub struct AccountsController<R>(Arc<Service<R>>);

// Manual `Clone` (not derived): the controller is `Clone` for ANY `R` because it
// only holds an `Arc`; the derive would wrongly demand `R: Clone`.
impl<R> Clone for AccountsController<R> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<R: CredentialRepository> AccountsController<R> {
    /// Builds the controller over `repository` and the in-process ports. The
    /// composition root chooses the repository; the interface never names a
    /// concrete one.
    pub fn new(repository: R) -> Self {
        AccountsController(Arc::new(AccountsService {
            repository,
            ids: SystemIds,
            uow: DirectUnitOfWork,
            events: NoopEventPublisher,
        }))
    }
}

/// Derives the caller's tenant [`Scope`] from the request context, **fail-closed**
/// (SG1): an untenanted or anonymous caller gets an error, never a global/unscoped
/// write. The tenant comes from the authenticated `Principal` (server-side), never
/// from the request body (OWASP API1). Passed to the application so every statement
/// the use case runs is bound to this tenant by the DORM gateway.
fn scope_of(ctx: &Ctx) -> Result<Scope, UserError> {
    ctx.tenant()
        .and_then(Scope::tenant)
        .ok_or_else(|| UserError("caller has no tenant".to_owned()))
}

#[controller]
impl<R> Accounts for AccountsController<R>
where
    R: CredentialRepository + Send + Sync + 'static,
{
    // `UnitOfWork::transaction` returns a `Send` future (RPITIT with a `+ Send`
    // bound in `seiva-ports`), so the use-case future is `Send` in this axum
    // handler and is awaited directly — no boxing or `spawn_blocking`. (Argon2id
    // hashing runs inline here; offloading that CPU-bound step is a separate
    // performance concern, not a `Send` requirement.)
    //
    // The `&Ctx` is injected by `#[seiva::controller]` (skipped from the wire, ADR
    // 0006); we derive the tenant `Scope` from it here and hand the application a
    // `Scope`, keeping it HTTP-free.
    async fn create_user(&self, ctx: &Ctx, cmd: CreateUser) -> Result<UserDto, UserError> {
        self.0.create_user(scope_of(ctx)?, cmd).await
    }

    async fn get_user(&self, ctx: &Ctx, query: GetUser) -> Result<UserDto, UserError> {
        self.0.get_user(scope_of(ctx)?, query).await
    }
}

/// Wraps an [`AccountsService`] and exposes a plain-Rust handler surface.
///
/// Retained for the CLI (`seiva user create`) and the composition test, which
/// drive the use case without the HTTP transport. It wraps the service's INHERENT
/// `create_user`/`get_user` (not the [`Accounts`] facade, which carries the HTTP
/// `&Ctx`): a caller without a transport passes the tenant [`Scope`] explicitly —
/// a tenant for a per-tenant call, or [`Scope::Global`] for genuine cross-tenant
/// admin work. It maps the facade's [`UserError`] to a plain `String` so its
/// callers keep the simple error shape they had before the facade gained a typed
/// wire error.
pub struct AccountsApi<R, G, U, E> {
    /// The wrapped application service.
    pub service: AccountsService<R, G, U, E>,
}

impl<R, G, U, E> AccountsApi<R, G, U, E>
where
    R: CredentialRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    /// Wraps the given service.
    pub fn new(service: AccountsService<R, G, U, E>) -> Self {
        Self { service }
    }

    /// Creates a user under `scope` (the caller's tenant), returning the public
    /// [`UserDto`] or the module's error message.
    pub async fn create_user(
        &self,
        scope: Scope,
        username: String,
        password: String,
    ) -> Result<UserDto, String> {
        self.service
            .create_user(scope, CreateUser { username, password })
            .await
            .map_err(|err| err.0)
    }

    /// Looks a user up by username under `scope`, returning the public [`UserDto`]
    /// or the module's error message.
    pub async fn get_user(&self, scope: Scope, username: String) -> Result<UserDto, String> {
        self.service
            .get_user(scope, GetUser { username })
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

    /// The composition the real `apps/` root performs: the generated
    /// `accounts_router` over a controller wired to the in-memory repository.
    /// `InMemoryCredentialRepository` is a dev-dependency — the interface's
    /// production code never names the infrastructure crate.
    fn demo_router() -> axum::Router {
        accounts_router(AccountsController::new(InMemoryCredentialRepository::new()))
    }

    // --- The composed in-memory service, driven as plain Rust via AccountsApi --

    /// Builds a ready `AccountsApi` from the in-memory infrastructure wired to
    /// the `seiva-testing` port doubles — the composition the real `apps/`
    /// composition root performs, proven end to end here.
    fn composed_api() -> AccountsApi<
        InMemoryCredentialRepository,
        SequenceIdGenerator,
        InMemoryUnitOfWork,
        RecordingEventPublisher,
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
        let scope = Scope::tenant("acme").expect("a non-empty tenant");

        let created = api
            .create_user(scope.clone(), "alice".into(), "hunter2".into())
            .await
            .unwrap();
        assert_eq!(
            created,
            UserDto {
                id: 1,
                username: "alice".into()
            }
        );

        let fetched = api.get_user(scope, "alice".into()).await.unwrap();
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

        let response = demo_router().oneshot(request).await.unwrap();

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
            .extension(Principal::in_tenant("user", "acme", ["accounts.create"]))
            .body(Body::from(r#"{"username":"alice","password":"hunter2"}"#))
            .unwrap();

        let response = demo_router().oneshot(request).await.unwrap();

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
    async fn create_route_without_a_tenant_is_rejected_fail_closed() {
        // The caller is authenticated and holds `accounts.create`, but is
        // untenanted: `scope_of` refuses to derive a `Scope`, so the write never
        // runs under a global/unscoped transaction (SG1 fail-closed).
        let request = Request::builder()
            .method("POST")
            .uri("/v1/accounts/create_user")
            .header("content-type", "application/json")
            .extension(Principal::new("user", ["accounts.create"]))
            .body(Body::from(r#"{"username":"alice","password":"hunter2"}"#))
            .unwrap();

        let response = demo_router().oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.headers()["content-type"], PROBLEM_JSON);
        let body = json_body(response.into_body()).await;
        assert_eq!(body["status"], 422);
        assert!(body["detail"].as_str().unwrap().contains("tenant"));
    }

    #[tokio::test]
    async fn create_route_invalid_body_is_422_problem() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/accounts/create_user")
            .header("content-type", "application/json")
            .extension(Principal::in_tenant("user", "acme", ["accounts.create"]))
            .body(Body::from(r#"{"username":"   ","password":"hunter2"}"#))
            .unwrap();

        let response = demo_router().oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.headers()["content-type"], PROBLEM_JSON);
        let body = json_body(response.into_body()).await;
        assert_eq!(body["status"], 422);
        assert!(body["detail"].as_str().unwrap().contains("username"));
    }

    #[tokio::test]
    async fn create_then_get_round_trips_through_the_real_use_case() {
        // One router instance so both requests share the in-memory repository.
        let app = demo_router();

        let create = Request::builder()
            .method("POST")
            .uri("/v1/accounts/create_user")
            .header("content-type", "application/json")
            .extension(Principal::in_tenant("user", "acme", ["accounts.create"]))
            .body(Body::from(r#"{"username":"alice","password":"hunter2"}"#))
            .unwrap();
        let response = app.clone().oneshot(create).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let get = Request::builder()
            .method("POST")
            .uri("/v1/accounts/get_user")
            .header("content-type", "application/json")
            .extension(Principal::in_tenant("user", "acme", ["accounts.read"]))
            .body(Body::from(r#"{"username":"alice"}"#))
            .unwrap();
        let response = app.oneshot(get).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response.into_body()).await;
        assert_eq!(body["username"], "alice");
    }
}
