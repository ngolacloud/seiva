//! The deny-by-default authentication middleware (ADR 0006).
//!
//! It runs ahead of every route's [`permission_guard`](crate::permission_guard),
//! turning an `Authorization: Bearer <token>` into the [`Principal`] the guard
//! and [`Scoped`](crate::Scoped) read:
//!
//! - **No credential header** → [`Principal::anonymous`] is inserted, so
//!   public routes (`/health`, `/docs`, `/openapi.json`) still serve and a
//!   guarded route's `permission_guard` answers `403`.
//! - **A present-but-invalid credential** (bad signature, wrong `aud`/`iss`,
//!   expired, unknown/revoked key, disallowed algorithm, or an empty/garbled
//!   header) → `401` RFC 9457 [`Problem`] with a `WWW-Authenticate` header. The
//!   request never reaches the route.
//!
//! Two credential sources, each with its own authenticator, run side by side
//! (ADR 0006): an opaque `X-API-Key` (checked first) and `Authorization: Bearer`.
//! The [`Authenticator`] is the only authority on identity; the middleware never
//! trusts a header or body field to set the subject, tenant or permissions.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use seiva_identity::{Authenticator, Principal};

use crate::problem::{Problem, ProblemResponse};
use crate::request_id::RequestIdExt;

/// The `X-API-Key` request header carrying an opaque API key.
static X_API_KEY: HeaderName = HeaderName::from_static("x-api-key");

/// The authenticators the [`authn`] middleware consults: the `Authorization:
/// Bearer` path (JWT or static token) and, optionally, an opaque `X-API-Key` path
/// running alongside it (ADR 0006).
///
/// Build it with [`Authn::bearer_only`] (the form every `router_with_*` uses) or
/// [`Authn::with_api_key`] to also admit `X-API-Key` callers, then mount with
/// [`axum::middleware::from_fn_with_state`]`(authn_state, authn)`.
#[derive(Clone)]
pub struct Authn {
    bearer: Arc<dyn Authenticator>,
    api_key: Option<Arc<dyn Authenticator>>,
}

impl Authn {
    /// Authenticate only `Authorization: Bearer` (no `X-API-Key` path). An
    /// `X-API-Key` header, if any, is ignored and the request falls through to the
    /// bearer/anonymous path.
    #[must_use]
    pub fn bearer_only(bearer: Arc<dyn Authenticator>) -> Self {
        Self {
            bearer,
            api_key: None,
        }
    }

    /// Authenticate `X-API-Key` against `api_key` **first**, then
    /// `Authorization: Bearer` against `bearer`. The two are independent
    /// mechanisms; a request presenting neither is the anonymous principal.
    #[must_use]
    pub fn with_api_key(bearer: Arc<dyn Authenticator>, api_key: Arc<dyn Authenticator>) -> Self {
        Self {
            bearer,
            api_key: Some(api_key),
        }
    }
}

/// Middleware that authenticates the caller and inserts a [`Principal`].
///
/// Reads `X-API-Key` first (when an API-key authenticator is configured), then
/// `Authorization: Bearer`; a request with neither becomes
/// [`Principal::anonymous`]. Mount it with
/// [`axum::middleware::from_fn_with_state`]`(authn_state, authn)`, where
/// `authn_state` is an [`Authn`]. [`router_with_auth`](crate::router_with_auth)
/// does this for the whole app.
pub async fn authn(State(authn): State<Authn>, mut request: Request, next: Next) -> Response {
    // 1. `X-API-Key` takes precedence, but only when an API-key authenticator is
    //    configured — otherwise the header is unknown and we fall through.
    if let Some(api_key_auth) = authn.api_key.as_ref() {
        if let Some(key) = request
            .headers()
            .get(&X_API_KEY)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .map(str::to_owned)
        {
            return match api_key_auth.authenticate(&key) {
                Ok(principal) => {
                    request.extensions_mut().insert(principal);
                    next.run(request).await
                }
                Err(_) => unauthorized(request, "ApiKey"),
            };
        }
    }

    // 2. `Authorization: Bearer`.
    let presented = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    let Some(header) = presented else {
        // No credential: the anonymous principal. The per-route guard, not this
        // layer, is what denies a guarded route — so public routes keep serving.
        request.extensions_mut().insert(Principal::anonymous());
        return next.run(request).await;
    };

    // A credential WAS presented: it must authenticate, or the request is refused.
    let bearer = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))
        .unwrap_or("")
        .trim();

    match authn.bearer.authenticate(bearer) {
        Ok(principal) => {
            request.extensions_mut().insert(principal);
            next.run(request).await
        }
        Err(_) => unauthorized(request, "Bearer"),
    }
}

/// Build the `401` RFC 9457 response for a present-but-invalid credential, tagged
/// with the `WWW-Authenticate` scheme the caller should use (`Bearer` or `ApiKey`).
fn unauthorized(request: Request, scheme: &'static str) -> Response {
    let request_id = request.extensions().request_id();
    let mut response = ProblemResponse(
        Problem::unauthorized("the presented credential did not authenticate")
            .with_instance(request.uri().path().to_owned())
            .with_request_id_opt(request_id),
    )
    .into_response();
    response
        .headers_mut()
        .insert(WWW_AUTHENTICATE, HeaderValue::from_static(scheme));
    response
}
