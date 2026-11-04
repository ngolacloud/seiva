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

/// The authenticated caller, the tenant it belongs to, and the permissions it
/// holds. Placed in the request extensions by an authentication layer upstream
/// of [`permission_guard`].
///
/// # Function-level vs object-level authorization
///
/// [`permission_guard`] is **function-level** (OWASP API5): it checks the caller
/// holds the named permission for the route. It cannot be **object-level** (OWASP
/// API1 / BOLA), because the middleware runs before the handler loads the
/// resource, so it does not yet know which tenant owns it. Object-level checks
/// belong where the resource is known: once a handler (or the application layer)
/// has loaded a record, it calls [`Principal::require_owns`] with the record's
/// tenant. [`tenant`](Self::tenant) is the building block both layers share.
#[derive(Debug, Clone, Default)]
pub struct Principal {
    /// A stable identifier for the caller (subject). Empty for the anonymous
    /// principal.
    pub subject: String,
    /// The tenant the caller belongs to, if any. `None` for the anonymous
    /// principal and for single-tenant deployments that do not scope by tenant.
    /// Compared against a resource's tenant by [`owns`](Self::owns).
    pub tenant: Option<String>,
    /// The permissions the caller holds, compared verbatim against a facade
    /// method's `requires`.
    pub permissions: BTreeSet<String>,
}

impl Principal {
    /// The anonymous principal: no subject, no tenant, no permissions.
    pub fn anonymous() -> Self {
        Self::default()
    }

    /// A principal with the given subject holding `permissions`, belonging to no
    /// particular tenant. Use [`in_tenant`](Self::in_tenant) to scope it.
    pub fn new<I, P>(subject: impl Into<String>, permissions: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<String>,
    {
        Self {
            subject: subject.into(),
            tenant: None,
            permissions: permissions.into_iter().map(Into::into).collect(),
        }
    }

    /// A principal scoped to `tenant`, holding `permissions`.
    pub fn in_tenant<I, P>(
        subject: impl Into<String>,
        tenant: impl Into<String>,
        permissions: I,
    ) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<String>,
    {
        Self {
            subject: subject.into(),
            tenant: Some(tenant.into()),
            permissions: permissions.into_iter().map(Into::into).collect(),
        }
    }

    /// Whether the caller holds `permission`.
    pub fn has(&self, permission: &str) -> bool {
        self.permissions.contains(permission)
    }

    /// Whether the caller owns a resource belonging to `resource_tenant`.
    ///
    /// True only when the caller has a tenant and it matches `resource_tenant`.
    /// An anonymous or tenant-less principal owns nothing, so this is a safe
    /// default: a missing tenant never grants cross-tenant access.
    #[must_use]
    pub fn owns(&self, resource_tenant: &str) -> bool {
        self.tenant.as_deref() == Some(resource_tenant)
    }

    /// Asserts the caller owns a resource in `resource_tenant`, returning a `403`
    /// [`Problem`] otherwise. The object-level (API1 / BOLA) counterpart to the
    /// function-level [`permission_guard`]: call it from a handler or the
    /// application layer once the resource's tenant is known.
    ///
    /// # Errors
    ///
    /// Returns a `403` [`Problem`] when the caller does not own the resource.
    pub fn require_owns(&self, resource_tenant: &str) -> Result<(), Problem> {
        if self.owns(resource_tenant) {
            Ok(())
        } else {
            Err(Problem::forbidden(
                "caller does not own the requested resource",
            ))
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn tenant_scoped_principal_owns_only_its_own_tenant() {
        let p = Principal::in_tenant("alice", "acme", ["notes.read"]);
        assert!(p.owns("acme"));
        assert!(!p.owns("globex"));
        assert!(p.has("notes.read"));
        assert_eq!(p.tenant.as_deref(), Some("acme"));
    }

    #[test]
    fn tenantless_and_anonymous_principals_own_nothing() {
        // A permission alone must never imply ownership of a tenant's resource.
        assert!(!Principal::new("svc", ["notes.read"]).owns("acme"));
        assert!(!Principal::anonymous().owns("acme"));
    }

    #[test]
    fn require_owns_allows_same_tenant_and_forbids_cross_tenant() {
        let p = Principal::in_tenant("alice", "acme", ["notes.read"]);
        assert!(p.require_owns("acme").is_ok());

        let denied = p.require_owns("globex").unwrap_err();
        assert_eq!(denied.status, StatusCode::FORBIDDEN);
    }
}
