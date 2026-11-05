//! The framework request context a facade method may receive as `&Ctx`.
//!
//! A facade/controller method can take a `&Ctx` parameter (recognised and
//! **skipped from the wire** by `#[seiva::controller]`, ADR 0006): the transport
//! injects it, carrying the authenticated [`Principal`]. A handler reads the
//! caller's tenant and permissions from it to scope data **server-side** — the
//! tenant is never taken from the request body (OWASP API1). `Ctx` is itself an
//! axum extractor (infallible: no principal in scope → [`Principal::anonymous`]),
//! so the generated handler builds one from the request's extensions.

use std::convert::Infallible;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use seiva_identity::Principal;

/// The request context handed to a facade method's `&Ctx` parameter: the
/// authenticated caller, from which a handler reads the tenant and permissions.
#[derive(Debug, Clone)]
pub struct Ctx {
    principal: Principal,
}

impl Ctx {
    /// Builds a context around `principal`.
    #[must_use]
    pub fn new(principal: Principal) -> Self {
        Self { principal }
    }

    /// The authenticated caller.
    #[must_use]
    pub fn principal(&self) -> &Principal {
        &self.principal
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

    /// Whether the caller holds `permission`.
    #[must_use]
    pub fn has(&self, permission: &str) -> bool {
        self.principal.has(permission)
    }
}

impl<S: Sync> FromRequestParts<S> for Ctx {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Ctx::new(
            parts
                .extensions
                .get::<Principal>()
                .cloned()
                .unwrap_or_else(Principal::anonymous),
        ))
    }
}
