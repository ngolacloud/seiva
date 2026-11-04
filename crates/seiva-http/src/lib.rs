//! Delonix Seiva — HTTP transport (first cut).
//!
//! This crate serves a module's facades over HTTP with axum/tower. It is the
//! foundation the per-facade controllers (a later `#[controller]` macro) build
//! on; this first cut provides the pieces and wires **one** facade method by hand
//! to prove them end to end:
//!
//! - [`problem`] — RFC 9457 `application/problem+json` errors ([`Problem`]).
//! - [`valid`] — a typed, validated JSON body extractor ([`Valid`], [`Validate`]).
//! - [`guard`] — a permission guard driven by a facade
//!   [`MethodDescriptor`](delonix_seiva::contract::MethodDescriptor)
//!   ([`permission_guard`], [`Principal`], [`RequiredPermission`]).
//! - [`health`] — `/health/live` and `/health/ready` over a
//!   `Starting -> Ready -> Draining` lifecycle ([`Health`]).
//! - [`mod@request_id`] — per-request correlation ([`RequestId`]).
//!
//! It is **DORM-free**: it depends only on the contract crate (with the data
//! layer off) and the trait-only ports. `/openapi.json` is generated from the
//! mounted facade descriptors by `seiva-openapi` ([`openapi_document`]).
//!
//! # The demonstration
//!
//! [`router`] mounts the `Notes` facade's `create` method as
//! `POST /{version}/notes`, where both the path version and the permission the
//! route enforces are read from
//! [`NotesFacade::DESCRIPTOR`](notes_facade::NotesFacade) — not hard-coded. See
//! [`notes_router`].

#![forbid(unsafe_code)]
// `Problem` is a rich RFC 9457 payload (several owned strings) and handlers
// return it by value on the error path — that is the design, not a footgun to
// box away, so the large-Err lint is allowed crate-wide.
#![allow(clippy::result_large_err)]

pub mod guard;
pub mod health;
pub mod problem;
pub mod request_id;
pub mod security;
pub mod valid;

use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use delonix_seiva::contract::Facade;
use notes_facade::NotesFacade;

pub use guard::{permission_guard, Principal, RequiredPermission};
pub use health::{health_router, Health, HealthState};
pub use problem::{Problem, PROBLEM_JSON};
pub use request_id::{request_id, RequestId, REQUEST_ID_HEADER};
pub use security::{cors_layer, security_headers, HttpSecurity};
pub use valid::{Valid, Validate, ValidationError};

/// The default maximum request body size (1 MiB). Oversized bodies are rejected
/// before a handler runs. Mounted as [`DefaultBodyLimit`] in [`router`].
pub const DEFAULT_BODY_LIMIT_BYTES: usize = 1024 * 1024;

/// The wire body for `POST /{version}/notes`: the interface layer owns this
/// shape, decoupled from the facade's `CreateNote` DTO (which carries no serde
/// derives). [`Validate`] enforces a non-empty, bounded title.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateNoteBody {
    /// The note's title.
    pub title: String,
}

impl Validate for CreateNoteBody {
    fn validate(&self) -> Result<(), ValidationError> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err(ValidationError::new("title must not be empty"));
        }
        if title.chars().count() > 200 {
            return Err(ValidationError::new("title must be at most 200 characters"));
        }
        Ok(())
    }
}

/// The wire response for a created note, mirroring the facade's `NoteDto`.
#[derive(Debug, Clone, Serialize)]
pub struct NoteResponse {
    /// The new note's identifier.
    pub id: u128,
    /// The note's title.
    pub title: String,
}

/// Handler for `create`: validates the body, crosses into the facade's DTOs to
/// prove the wiring, and returns the created note as `201` JSON.
///
/// No application/infrastructure is linked in this transport-only crate, so the
/// result is synthesised rather than produced by the real use case. A later
/// phase injects the facade implementation; the transport shape stays the same.
async fn create_note(Valid(body): Valid<CreateNoteBody>) -> Result<Response, Problem> {
    // Cross into the real facade DTOs: this is the mapping a generated controller
    // will perform, proving the transport depends on the facade contract.
    let command = notes_facade::CreateNote { title: body.title };
    let dto = notes_facade::NoteDto {
        id: 1,
        title: command.title,
    };

    let response = NoteResponse {
        id: dto.id,
        title: dto.title,
    };
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// Builds the `Notes` sub-router, reading the route **version** and the
/// **required permission** from
/// [`NotesFacade::DESCRIPTOR`](notes_facade::NotesFacade). Nothing here names
/// `"v1"` or `"notes.create"` literally: change the facade and the route follows.
pub fn notes_router() -> Router {
    let descriptor = NotesFacade::DESCRIPTOR;
    let create = descriptor
        .method("create")
        .expect("the Notes facade declares a `create` method");

    let path = format!("/{}/notes", descriptor.version);

    Router::new()
        .route(&path, post(create_note))
        .layer(from_fn_with_state(
            RequiredPermission(create.requires),
            permission_guard,
        ))
}

/// Builds the service's OpenAPI 3.1 document from the mounted facade
/// descriptors with [`seiva_openapi`]. Nothing here is hand-written: the paths,
/// operations, permissions and schemas follow the descriptors, so adding a
/// facade adds its operations. The service mounts the `Notes` facade, so its
/// descriptor is the single input today.
pub fn openapi_document() -> serde_json::Value {
    seiva_openapi::openapi_document(
        seiva_openapi::ApiInfo {
            title: "Delonix Seiva service".to_string(),
            version: delonix_seiva::VERSION.to_string(),
        },
        &[NotesFacade::DESCRIPTOR],
    )
}

/// `GET /openapi.json` — serves the generated [`openapi_document`].
async fn openapi_handler() -> Json<serde_json::Value> {
    Json(openapi_document())
}

/// Builds the full application router with the framework's **default** security
/// posture ([`HttpSecurity::default`]: secure response headers on, same-origin
/// only — no CORS). Equivalent to [`router_with_security`] with the default
/// configuration; kept as the zero-config entrypoint.
pub fn router(health: Health) -> Router {
    router_with_security(health, &HttpSecurity::default())
}

/// Builds the full application router: the `Notes` demonstration, the health
/// endpoints and the OpenAPI stub, wrapped (innermost to outermost) with the
/// security response headers, an optional CORS layer, a body-size limit and the
/// [`request_id()`] middleware (outermost, so every response — problems included —
/// carries a correlation id).
///
/// `security` controls the transport-level API8 defences: the response headers
/// are always applied, and CORS is mounted only when
/// [`HttpSecurity::cors_allowed_origins`] is non-empty (otherwise the browser
/// enforces same-origin).
pub fn router_with_security(health: Health, security: &HttpSecurity) -> Router {
    let mut app = Router::new()
        .merge(notes_router())
        .merge(health_router(health))
        .route("/openapi.json", get(openapi_handler))
        .layer(from_fn(security_headers))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT_BYTES));

    if let Some(cors) = cors_layer(security) {
        app = app.layer(cors);
    }

    app.layer(from_fn(request_id))
}
