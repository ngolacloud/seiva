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
// `Problem` is a rich RFC 9457 payload (several owned strings) and handlers
// return it by value on the error path — that is the design, not a footgun to
// box away, so the large-Err lint is allowed crate-wide.
#![allow(clippy::result_large_err)]

pub mod docs;
pub mod guard;
pub mod health;
pub mod problem;
pub mod rate_limit;
pub mod request_id;
pub mod security;
pub mod valid;

use axum::extract::DefaultBodyLimit;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::get;
use axum::{Json, Router};

pub use docs::{docs_handler, DOCS_HTML};
pub use guard::{permission_guard, Principal, RequiredPermission};
pub use health::{health_router, Health, HealthState};
pub use problem::{Problem, PROBLEM_JSON};
pub use rate_limit::{rate_limit, RateLimit, RateLimiter};
pub use request_id::{request_id, RequestId, REQUEST_ID_HEADER};
pub use security::{cors_layer, security_headers, HttpSecurity};
pub use valid::{Valid, Validate, ValidationError};

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
/// `/docs` — wrapped (innermost to outermost) with the security response headers,
/// an optional CORS layer, a body-size limit, the rate limiter and the
/// [`request_id()`] middleware (outermost, so every response — problems included —
/// carries a correlation id).
///
/// It mounts no module route; the composition root merges each module's generated
/// controller router into the result. `security` controls the transport-level
/// API8 defences: the response headers are always applied, and CORS is mounted
/// only when [`HttpSecurity::cors_allowed_origins`] is non-empty (otherwise the
/// browser enforces same-origin).
pub fn router_with_security(health: Health, security: &HttpSecurity) -> Router {
    let mut app = Router::new()
        .merge(health_router(health))
        .route("/openapi.json", get(openapi_handler))
        .route("/docs", get(docs::docs_handler))
        .layer(from_fn(security_headers))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT_BYTES));

    if let Some(cors) = cors_layer(security) {
        app = app.layer(cors);
    }

    if let Some(limit) = security.rate_limit {
        app = app.layer(from_fn_with_state(RateLimiter::new(limit), rate_limit));
    }

    app.layer(from_fn(request_id))
}
