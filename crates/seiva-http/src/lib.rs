//! Delonix Seiva — the HTTP transport.
//!
//! This crate provides the transport-level pieces a module's generated
//! controller builds on. It is **demo-free and DORM-free**: it links no example
//! module and no data layer, so it is a publishable building block. A module's
//! `#[seiva::controller]` router and the composition root (`seiva-server` or a
//! project's `apps/<name>-api`) merge the module routes into [`router`].
//!
//! - [`problem`] — RFC 9457 `application/problem+json` errors ([`Problem`]).
//! - [`valid`] — a typed, validated JSON body extractor ([`Valid`], [`Validate`]).
//! - [`guard`] — a descriptor-driven permission guard ([`permission_guard`],
//!   [`Principal`], [`RequiredPermission`]) plus tenant-ownership checks.
//! - [`security`] — secure-by-default response headers and configurable CORS.
//! - [`mod@rate_limit`] — a per-client request rate limiter.
//! - [`health`] — `/health/live` and `/health/ready` over a
//!   `Starting -> Ready -> Draining` lifecycle ([`Health`]).
//! - [`docs`] — an interactive `/docs` API reference over `/openapi.json`.
//! - [`mod@request_id`] — per-request correlation ([`RequestId`]).
//!
//! [`router`] assembles the framework surface — health, `/openapi.json`
//! (generated from every facade linked into the binary) and `/docs` — wrapped in
//! the security headers, optional CORS, the body-size limit, the rate limiter and
//! the request-id middleware. It mounts no module route itself; the composition
//! root merges those in.

#![forbid(unsafe_code)]

pub mod audit;
pub mod authn;
pub mod ctx;
pub mod docs;
pub mod guard;
pub mod health;
pub mod idempotency;
pub mod problem;
pub mod rate_limit;
pub mod request_id;
pub mod security;
pub mod valid;

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::get;
use axum::{Json, Router};

pub use audit::audit;
pub use authn::authn;
pub use ctx::Ctx;
pub use docs::{docs_handler, DOCS_HTML};
pub use guard::{permission_guard, PrincipalExt, RequiredPermission, Scoped};
pub use health::{
    health_router, health_router_with_dependency, Health, HealthState, ReadinessCheck,
};
pub use idempotency::{
    fingerprint, in_progress_problem, mismatch_problem, scoped_key, IdempotencyKey,
    OptionalIdempotencyKey, IDEMPOTENCY_HEADER, MAX_KEY_LEN,
};
pub use problem::{Problem, PROBLEM_JSON};
pub use rate_limit::{
    rate_limit, rate_limit_with_store, DistributedRateLimit, RateLimit, RateLimiter,
};
pub use request_id::{request_id, RequestId, REQUEST_ID_HEADER};
pub use security::{cors_layer, security_headers, HttpSecurity};
pub use seiva_ports::{AuditEntry, AuditSink};
pub use seiva_ports::{RateLimitDecision, RateLimitStore};
pub use valid::{Valid, Validate, ValidationError};
// The identity model is owned by `seiva-identity` (transport-free, shared with
// MCP); re-exported here so a `seiva_http::Principal` import keeps working.
pub use seiva_identity::{
    AuthError, Authenticator, JwtAuthenticator, Principal, TokenAuthenticator,
};

/// The default maximum request body size (1 MiB). Oversized bodies are rejected
/// before a handler runs. Mounted as [`DefaultBodyLimit`] in [`router`].
pub const DEFAULT_BODY_LIMIT_BYTES: usize = 1024 * 1024;

/// Builds the service's OpenAPI 3.1 document from **every facade linked into the
/// binary**, discovered through [`delonix_seiva::contract::registered_facades`]
/// (each `#[seiva::facade]` registers itself in a link-time inventory). Nothing
/// here is hand-written and no central list is kept: a module mounted into the
/// runtime contributes its operations automatically (OWASP API9). This crate
/// links no facade of its own, so a plain `seiva-http` binary documents nothing;
/// the full runtime (see `seiva-server`) documents every mounted module.
pub fn openapi_document() -> serde_json::Value {
    let facades = delonix_seiva::contract::registered_facades();
    seiva_openapi::openapi_document(
        seiva_openapi::ApiInfo {
            title: "Delonix Seiva service".to_string(),
            version: delonix_seiva::VERSION.to_string(),
        },
        &facades,
    )
}

/// `GET /openapi.json` — serves the generated [`openapi_document`].
async fn openapi_handler() -> Json<serde_json::Value> {
    Json(openapi_document())
}

/// Builds the framework router with the **default** security posture
/// ([`HttpSecurity::default`]: secure response headers on, same-origin only — no
/// CORS). Equivalent to [`router_with_security`] with the default configuration;
/// the zero-config entrypoint.
pub fn router(health: Health) -> Router {
    router_with_security(health, &HttpSecurity::default())
}

/// Builds the framework router — the health endpoints, `/openapi.json` and
/// `/docs`. Request flows through the layers outermost-first:
/// [`request_id()`] → [`security_headers`] → the rate limiter → the optional CORS
/// layer → the body-size limit → the route.
///
/// `security_headers` and `request_id` are the **outermost** layers on purpose:
/// every response carries them even when an outer layer short-circuits before the
/// route runs — a rate-limiter `429` or a CORS preflight included. (With the
/// headers mounted inside the rate limiter, a `429` went out naked — no CSP, no
/// `nosniff`, no HSTS — measured end-to-end.)
///
/// It mounts no module route; the composition root merges each module's generated
/// controller router into the result. `security` controls the transport-level
/// API8 defences: the response headers are always applied, and CORS is mounted
/// only when [`HttpSecurity::cors_allowed_origins`] is non-empty (otherwise the
/// browser enforces same-origin).
pub fn router_with_security(health: Health, security: &HttpSecurity) -> Router {
    router_with_auth(health, security, None)
}

/// Builds the framework router under `security` and an optional authentication
/// layer: when `authenticator` is `Some`, every request is authenticated (ADR
/// 0006) ahead of any route's [`permission_guard`] — a missing bearer becomes
/// [`Principal::anonymous`] (public routes still serve, guarded routes `403`),
/// a present-but-invalid one is a `401`. `None` mounts no authn layer (no caller
/// is ever authenticated, so guarded routes answer `403`): the composition root
/// supplies an [`Authenticator`] to actually admit callers.
///
/// Layer order, outermost-first: [`request_id()`] → [`security_headers`] →
/// [`authn()`] → the rate limiter → CORS → the body-size limit → the route.
/// `authn` sits inside the header layers (so a `401` still carries them) but
/// outside the routes (so it runs before each route's guard).
pub fn router_with_auth(
    health: Health,
    security: &HttpSecurity,
    authenticator: Option<Arc<dyn Authenticator>>,
) -> Router {
    apply_framework_layers(framework_routes(health), security, authenticator, None)
}

/// Builds the full application router: the framework routes **plus the caller's
/// `modules`**, wrapped in the framework layers — the composition a real
/// `apps/<name>-api` needs.
///
/// Why this exists: axum's [`Router::layer`](axum::Router::layer) wraps only the
/// routes already present, so `router_with_auth(..).merge(modules)` would leave
/// the module routes **outside** every middleware — no authn, no security
/// headers, no rate limit. Merging the modules *before* layering (what this does)
/// is the only order that applies the framework's guarantees to module routes.
/// Mount each module's generated controller router into `modules` and pass it here.
pub fn router_with_modules(
    health: Health,
    security: &HttpSecurity,
    authenticator: Option<Arc<dyn Authenticator>>,
    modules: Router,
) -> Router {
    apply_framework_layers(
        framework_routes(health).merge(modules),
        security,
        authenticator,
        None,
    )
}

/// Like [`router_with_modules`], but enforces the per-client rate limit with a
/// **distributed** store ([`DistributedRateLimit`], ADR 0015) instead of the
/// per-process limiter — the form that holds ONE budget across replicas behind a
/// load balancer. The distributed limiter takes the in-process one's exact layer
/// position (inside the security headers, so a `429` still carries them), so a
/// `429` keeps its correlation id and headers; `security.rate_limit` is ignored
/// when a distributed limiter is supplied. Set `security.trust_forwarded_for`
/// (and pass it through on the limiter) only behind a single trusted proxy.
pub fn router_with_modules_distributed(
    health: Health,
    security: &HttpSecurity,
    authenticator: Option<Arc<dyn Authenticator>>,
    modules: Router,
    distributed: DistributedRateLimit,
) -> Router {
    apply_framework_layers(
        framework_routes(health).merge(modules),
        security,
        authenticator,
        Some(distributed),
    )
}

/// The framework's own routes, unlayered: health, `/openapi.json` and `/docs`.
///
/// There is deliberately **no** `/admin` here: the admin console is an optional
/// extension (`seiva-console-admin`, ADR 0008), not part of the base. A backend
/// built without that crate answers `404` for `/admin`; a composition root that
/// wants it calls `seiva_console_admin::mount(router)` before serving.
fn framework_routes(health: Health) -> Router {
    Router::new()
        .merge(health_router(health))
        .route("/openapi.json", get(openapi_handler))
        .route("/docs", get(docs::docs_handler))
}

/// Wraps `app` (framework routes, and any merged module routes) in the transport
/// middleware. Layer order, outermost-first: [`request_id()`] →
/// [`security_headers`] → [`authn()`] → the rate limiter → CORS → the body-size
/// limit → the route. `authn` sits inside the header layers (so a `401` still
/// carries them) but outside the routes (so it runs before each route's guard).
fn apply_framework_layers(
    app: Router,
    security: &HttpSecurity,
    authenticator: Option<Arc<dyn Authenticator>>,
    distributed: Option<DistributedRateLimit>,
) -> Router {
    let mut app = app.layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT_BYTES));

    if let Some(cors) = cors_layer(security) {
        app = app.layer(cors);
    }

    // The distributed limiter, when supplied, takes the in-process one's place at
    // this exact layer position; otherwise the per-process limiter runs if the
    // security config enables it.
    if let Some(dist) = distributed {
        app = app.layer(from_fn_with_state(dist, rate_limit_with_store));
    } else if let Some(limit) = security.rate_limit {
        let limiter = RateLimiter::new(limit).trusting_forwarded_for(security.trust_forwarded_for);
        app = app.layer(from_fn_with_state(limiter, rate_limit));
    }

    if let Some(auth) = authenticator {
        app = app.layer(from_fn_with_state(auth, authn));
    }

    // Outermost: so a short-circuited response (a 429, a 401, a CORS preflight)
    // still leaves through the security headers and gets a correlation id.
    app.layer(from_fn(security_headers))
        .layer(from_fn(request_id))
}
