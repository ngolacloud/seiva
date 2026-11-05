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

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use seiva_identity::Principal;

use crate::problem::Problem;
use crate::request_id::RequestIdExt;

/// HTTP-flavoured object-level authorization helpers on the transport-free
/// [`Principal`] (which lives in `seiva-identity`). These return a `403`
/// [`Problem`], so they stay in the HTTP crate rather than in the identity crate.
///
/// # Function-level vs object-level authorization
///
/// [`permission_guard`] is **function-level** (OWASP API5): it checks the caller
/// holds the named permission for the route. It cannot be **object-level** (OWASP
/// API1 / BOLA), because the middleware runs before the handler loads the
/// resource, so it does not yet know which tenant owns it. Object-level checks
/// belong where the resource is known: once a handler (or the application layer)
/// has loaded a record, it calls [`require_owns`](PrincipalExt::require_owns)
/// with the record's tenant — or, fail-closed, wraps it in a [`Scoped`].
pub trait PrincipalExt {
    /// Asserts the caller owns a resource in `resource_tenant`, returning a `403`
    /// [`Problem`] otherwise. The advisory object-level (API1 / BOLA) counterpart
    /// to the function-level [`permission_guard`]; [`Scoped`] is the fail-closed one.
    ///
    /// # Errors
    ///
    /// Returns a `403` [`Problem`] when the caller does not own the resource.
    fn require_owns(&self, resource_tenant: &str) -> Result<(), Problem>;
}

impl PrincipalExt for Principal {
    fn require_owns(&self, resource_tenant: &str) -> Result<(), Problem> {
        if self.owns(resource_tenant) {
            Ok(())
        } else {
            Err(Problem::forbidden(
                "caller does not own the requested resource",
            ))
        }
    }
}

/// A value that belongs to a tenant and cannot be read without proving the caller
/// owns it — the **fail-closed** object-level (OWASP API1 / BOLA) counterpart to
/// the advisory [`Principal::require_owns`] (ADR 0005).
///
/// Build it where the resource is loaded, from the record's own tenant column:
///
/// ```
/// # use seiva_http::{Principal, Scoped};
/// # struct NoteDto { title: String }
/// // In the use case / handler, right after the load:
/// let note = Scoped::new("acme", NoteDto { title: "hi".into() });
///
/// // The ONLY way to reach the value is to pass the ownership check:
/// let caller = Principal::in_tenant("alice", "acme", ["notes.read"]);
/// let dto = note.owned_by(&caller).expect("same tenant");
/// assert_eq!(dto.title, "hi");
/// ```
///
/// There is no `into_inner`, no public field and no `Deref`: a handler that
/// forgets the check cannot reach the resource to return it, so the omission is a
/// compile error rather than a silent cross-tenant leak. `#[must_use]` means even
/// dropping it unused warns.
#[must_use = "a Scoped value must be unwrapped with `owned_by` — dropping it skips the ownership check"]
pub struct Scoped<T> {
    tenant: String,
    value: T,
}

impl<T> Scoped<T> {
    /// Tags `value` as belonging to `tenant`. The caller passes the resource's
    /// own tenant (its database column, say), not the requester's.
    pub fn new(tenant: impl Into<String>, value: T) -> Self {
        Self {
            tenant: tenant.into(),
            value,
        }
    }

    /// The tenant this value belongs to, readable without unwrapping — for a log
    /// line or a metric label, never as a substitute for [`owned_by`](Self::owned_by).
    #[must_use]
    pub fn tenant(&self) -> &str {
        &self.tenant
    }

    /// Unwraps the value **iff** `principal` owns its tenant, returning a `403`
    /// [`Problem`] otherwise. This is the only accessor of the inner value, so
    /// the ownership check cannot be skipped.
    ///
    /// # Errors
    ///
    /// Returns a `403` [`Problem`] when the caller does not own the resource.
    pub fn owned_by(self, principal: &Principal) -> Result<T, Problem> {
        if principal.owns(&self.tenant) {
            Ok(self.value)
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

    #[test]
    fn scoped_unwraps_only_for_the_owning_tenant() {
        let owner = Principal::in_tenant("alice", "acme", ["notes.read"]);
        let scoped = Scoped::new("acme", "secret-note");
        assert_eq!(scoped.tenant(), "acme");
        assert_eq!(scoped.owned_by(&owner).unwrap(), "secret-note");
    }

    #[test]
    fn scoped_forbids_a_cross_tenant_caller() {
        let intruder = Principal::in_tenant("mallory", "globex", ["notes.read"]);
        let scoped = Scoped::new("acme", "secret-note");
        let denied = scoped.owned_by(&intruder).unwrap_err();
        assert_eq!(denied.status, StatusCode::FORBIDDEN);
    }

    #[test]
    fn scoped_forbids_a_tenantless_or_anonymous_caller() {
        // A permission alone (no tenant) must never unwrap a tenant's resource.
        assert!(Scoped::new("acme", 1)
            .owned_by(&Principal::new("svc", ["notes.read"]))
            .is_err());
        assert!(Scoped::new("acme", 1)
            .owned_by(&Principal::anonymous())
            .is_err());
    }
}
