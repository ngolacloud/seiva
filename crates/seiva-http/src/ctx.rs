//! The framework request context a facade method may receive as `&Ctx`.
//!
//! A facade/controller method can take a `&Ctx` parameter (recognised and
//! **skipped from the wire** by `#[seiva::controller]`, ADR 0006): the transport
//! injects it, carrying the authenticated [`Principal`]. A handler reads the
//! caller's tenant and permissions from it to scope data **server-side** — the
//! tenant is never taken from the request body (OWASP API1). `Ctx` is itself an
//! axum extractor (infallible: no principal in scope → [`Principal::anonymous`]),
//! so the generated handler builds one from the request's extensions.
//!
//! It also carries the request's operation coordinates (method + path) and the
//! optional, validated `Idempotency-Key` (ADR 0011), so a write handler can fold
//! the durable idempotency key + fingerprint **inside its transaction** without
//! the controller macro having to grow a new parameter.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use seiva_identity::Principal;
use seiva_ports::Scope;

use crate::idempotency::IdempotencyKey;

/// The request context handed to a facade method's `&Ctx` parameter: the
/// authenticated caller (tenant + permissions) plus the operation coordinates and
/// an optional idempotency key.
#[derive(Debug, Clone)]
pub struct Ctx {
    principal: Arc<Principal>,
    method: String,
    path: String,
    idempotency_key: Option<IdempotencyKey>,
}

impl Ctx {
    /// Builds a context around `principal` with no request coordinates and no
    /// idempotency key (the transport fills those in via the extractor; this
    /// constructor is what a direct caller or a test uses).
    #[must_use]
    pub fn new(principal: Principal) -> Self {
        Self::from_shared(Arc::new(principal))
    }

    /// Builds a context around an **already-shared** principal — the extractor's
    /// path. It reuses the `Arc<Principal>` the `authn` layer put in the request
    /// extensions instead of deep-cloning the subject, tenant and permission set:
    /// one refcount bump, no allocation (panel finding E2).
    #[must_use]
    pub(crate) fn from_shared(principal: Arc<Principal>) -> Self {
        Self {
            principal,
            method: String::new(),
            path: String::new(),
            idempotency_key: None,
        }
    }

    /// Attaches the request's method + path and an optional idempotency key — used
    /// by the extractor, and by a test that drives a handler directly.
    #[must_use]
    pub fn with_request(
        mut self,
        method: impl Into<String>,
        path: impl Into<String>,
        idempotency_key: Option<IdempotencyKey>,
    ) -> Self {
        self.method = method.into();
        self.path = path.into();
        self.idempotency_key = idempotency_key;
        self
    }

    /// The authenticated caller.
    #[must_use]
    pub fn principal(&self) -> &Principal {
        self.principal.as_ref()
    }

    /// The caller's tenant, if any — the value a handler scopes its queries by.
    /// `None` for an untenanted or anonymous caller (which must then see nothing
    /// tenant-scoped).
    #[must_use]
    pub fn tenant(&self) -> Option<&str> {
        self.principal.tenant.as_deref()
    }

    /// The caller's stable subject (empty for anonymous).
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.principal.subject
    }

    /// The data [`Scope`] this request runs under, derived **server-side** from the
    /// authenticated caller — never from the request body (OWASP API1). A caller
    /// bound to a tenant yields `Some(`[`Scope::Tenant`]`)`, so every `tenant_scoped`
    /// write in the use case is constrained to that tenant automatically.
    ///
    /// **Fail-closed (SG1, ADR 0029 BOLA):** a tenant-less or anonymous caller yields
    /// [`None`], *not* [`Scope::Global`]. The handler must treat `None` as a refusal
    /// (the `accounts`/`notes` `scope_of` shape: `ctx.scope().ok_or(..)?`) so a
    /// tenant-scoped command can never silently run cross-tenant. [`Scope::Global`] is
    /// a deliberate, audited cross-tenant choice a caller writes **explicitly** (the
    /// `seiva:allow-global` fitness-lint allowlist) — never a fallback this method
    /// hands out.
    #[must_use]
    pub fn scope(&self) -> Option<Scope> {
        self.principal.tenant.as_deref().and_then(Scope::tenant)
    }

    /// Whether the caller holds `permission`.
    #[must_use]
    pub fn has(&self, permission: &str) -> bool {
        self.principal.has(permission)
    }

    /// The request method (e.g. `POST`), empty when built without a request.
    #[must_use]
    pub fn method(&self) -> &str {
        &self.method
    }

    /// The request path (e.g. `/v1/catalogs`), empty when built without a request.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The validated `Idempotency-Key` the caller sent, if any. Idempotency is
    /// opt-in per request; absent (or malformed — see the extractor) means a plain,
    /// non-deduplicated call.
    #[must_use]
    pub fn idempotency_key(&self) -> Option<&IdempotencyKey> {
        self.idempotency_key.as_ref()
    }
}

impl<S: Sync> FromRequestParts<S> for Ctx {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let principal = parts
            .extensions
            .get::<Arc<Principal>>()
            .cloned()
            .unwrap_or_else(|| Arc::new(Principal::anonymous()));
        // Best-effort key: a present-but-malformed key is treated as absent (a
        // plain create), never stored — the validation still guards the storage
        // key. A handler that wants a hard `400` on a bad key uses the
        // `OptionalIdempotencyKey` extractor directly instead.
        let idempotency_key = parts
            .headers
            .get(crate::idempotency::IDEMPOTENCY_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(|raw| IdempotencyKey::parse(raw).ok());
        Ok(Ctx::from_shared(principal).with_request(
            parts.method.as_str().to_owned(),
            parts.uri.path().to_owned(),
            idempotency_key,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seiva_ports::Scope;

    #[test]
    fn a_tenant_bound_caller_scopes_to_its_tenant() {
        let ctx = Ctx::new(Principal::in_tenant("u1", "acme", ["orders:write"]));
        match ctx.scope() {
            Some(Scope::Tenant(t)) => assert_eq!(t.as_str(), "acme"),
            other => panic!("expected Some(Scope::Tenant(acme)), got {other:?}"),
        }
    }

    #[test]
    fn a_tenant_less_caller_fails_closed_not_global() {
        // Audit BOLA-1: an authenticated-but-tenant-less caller must yield `None`,
        // NEVER `Scope::Global` — the handler is forced to refuse (`.ok_or(..)?`).
        let authn = Ctx::new(Principal::new("u1", ["orders:write"]));
        assert_eq!(authn.scope(), None, "tenant-less must not map to a scope");

        let anon = Ctx::new(Principal::anonymous());
        assert_eq!(anon.scope(), None, "anonymous must not map to a scope");
    }
}
