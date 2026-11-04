//! The descriptor-driven permission guard.
//!
//! A facade's [`MethodDescriptor.requires`](delonix_seiva::contract::MethodDescriptor)
//! names the permission a caller must hold. [`permission_guard`] is a tower
//! middleware, parameterised by that string via [`RequiredPermission`], that
//! reads the [`Principal`] a request carries and returns a `403`
//! [`Problem`] when the permission is absent. An empty
//! requirement (`""`, an unguarded method) always passes.
//!
//! The principal reaches a request through its extensions (an authentication
//! layer inserts it; a test inserts it directly). A request with no principal is
//! treated as [`Principal::anonymous`] — holding nothing — so an unauthenticated
//! caller is denied, not admitted.

use std::collections::BTreeSet;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::problem::Problem;
use crate::request_id::RequestIdExt;

/// The authenticated caller and the permissions it holds. Placed in the request
/// extensions by an authentication layer upstream of [`permission_guard`].
#[derive(Debug, Clone, Default)]
pub struct Principal {
    /// A stable identifier for the caller (subject). Empty for the anonymous
    /// principal.
    pub subject: String,
    /// The permissions the caller holds, compared verbatim against a facade
    /// method's `requires`.
    pub permissions: BTreeSet<String>,
}

impl Principal {
    /// The anonymous principal: no subject, no permissions.
    pub fn anonymous() -> Self {
        Self::default()
    }

    /// A principal with the given subject holding `permissions`.
    pub fn new<I, P>(subject: impl Into<String>, permissions: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<String>,
    {
        Self {
            subject: subject.into(),
            permissions: permissions.into_iter().map(Into::into).collect(),
        }
    }

    /// Whether the caller holds `permission`.
    pub fn has(&self, permission: &str) -> bool {
        self.permissions.contains(permission)
    }
}

/// The permission a route requires, read from a facade method's descriptor. An
/// empty string marks an unguarded route. Used as the state of
/// [`permission_guard`].
#[derive(Debug, Clone, Copy)]
pub struct RequiredPermission(pub &'static str);

/// Middleware that enforces [`RequiredPermission`] against the request's
/// [`Principal`]. Mount it with
/// [`axum::middleware::from_fn_with_state`]`(RequiredPermission(perm), permission_guard)`.
pub async fn permission_guard(
    State(required): State<RequiredPermission>,
    request: Request,
    next: Next,
) -> Response {
    if required.0.is_empty() {
        return next.run(request).await;
    }

    let principal = request
        .extensions()
        .get::<Principal>()
        .cloned()
        .unwrap_or_else(Principal::anonymous);

    if principal.has(required.0) {
        return next.run(request).await;
    }

    let request_id = request.extensions().request_id();
    Problem::forbidden(format!("permission `{}` is required", required.0))
        .with_instance(request.uri().path().to_owned())
        .with_request_id_opt(request_id)
        .into_response()
}
