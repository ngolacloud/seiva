//! The deny-by-default authentication middleware (ADR 0006).
//!
//! It runs ahead of every route's [`permission_guard`](crate::permission_guard),
//! turning an `Authorization: Bearer <token>` into the [`Principal`] the guard
//! and [`Scoped`](crate::Scoped) read:
//!
//! - **No `Authorization` header** → [`Principal::anonymous`] is inserted, so
//!   public routes (`/health`, `/docs`, `/openapi.json`) still serve and a
//!   guarded route's `permission_guard` answers `403`.
//! - **A present-but-invalid bearer** (bad signature, wrong `aud`/`iss`,
//!   expired, unknown key, disallowed algorithm, or an empty/garbled header) →
//!   `401` RFC 9457 [`Problem`] with a `WWW-Authenticate: Bearer` header. The
//!   request never reaches the route.
//!
//! The [`Authenticator`] is the only authority on identity; the middleware never
//! trusts a header or body field to set the subject, tenant or permissions.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use seiva_identity::{Authenticator, Principal};

use crate::problem::Problem;
use crate::request_id::RequestIdExt;

/// Middleware that authenticates the bearer and inserts a [`Principal`].
///
/// Mount it with
/// [`axum::middleware::from_fn_with_state`]`(authenticator, authn)`, where
/// `authenticator` is an `Arc<dyn Authenticator>`.
/// [`router_with_auth`](crate::router_with_auth) does this for the whole app.
pub async fn authn(
    State(authenticator): State<Arc<dyn Authenticator>>,
    mut request: Request,
    next: Next,
) -> Response {
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

    match authenticator.authenticate(bearer) {
        Ok(principal) => {
            request.extensions_mut().insert(principal);
            next.run(request).await
        }
        Err(_) => {
            let request_id = request.extensions().request_id();
            let mut response = Problem::unauthorized("the bearer token did not authenticate")
                .with_instance(request.uri().path().to_owned())
                .with_request_id_opt(request_id)
                .into_response();
            response
                .headers_mut()
                .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
            response
        }
    }
}
