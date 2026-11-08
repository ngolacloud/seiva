//! Identity for Delonix Seiva: the authenticated caller ([`Principal`]) and how a
//! transport authenticates one ([`Authenticator`]).
//!
//! This crate is **transport-free** — it names no axum/HTTP type — so every
//! transport builds the same identity model on it: `seiva-http`'s middleware,
//! the `seiva-mcp` endpoint, a future gRPC server. The authorization layers
//! (`seiva-http`'s `permission_guard` and `Scoped<T>`) read the [`Principal`]
//! this crate produces (ADR 0006).
//!
//! # Deny by default
//!
//! An [`Authenticator`] turns a bearer token into a [`Principal`] or rejects it.
//! An empty or unrecognised bearer is a rejection, never a silent anonymous pass:
//! the caller decides (for HTTP, a *missing* token becomes [`Principal::anonymous`]
//! so public routes still serve, while a *present-but-invalid* one is a `401`).
//!
//! - [`TokenAuthenticator`] — a static token → principal map (dev/test).
//! - [`JwtAuthenticator`] — verifies an OIDC/JWT against an injected JWKS
//!   (RS256 / ES256, `iss`/`aud`/`exp`/`nbf`), mapping its claims to a principal.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::collections::BTreeSet;

pub mod jwt;

pub use jwt::JwtAuthenticator;

/// The authenticated caller: who they are, the tenant their requests are scoped
/// to (if any), and the permissions they hold.
///
/// Built only by an [`Authenticator`] from verified credentials — never from a
/// request argument. `tenant` of `None` means the caller is not tenant-scoped
/// (a tenant-wide or single-tenant deployment); an empty [`Principal`] is the
/// anonymous caller.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Principal {
    /// A stable, loggable identity for the caller (the token's subject). Never
    /// the bearer token itself. Empty for the anonymous principal.
    pub subject: String,
    /// The tenant every tenant-scoped resource is constrained to, if any.
    pub tenant: Option<String>,
    /// The permissions the caller holds, compared verbatim against a facade
    /// method's `requires`.
    pub permissions: BTreeSet<String>,
}

impl Principal {
    /// The anonymous principal: no subject, no tenant, no permissions.
    #[must_use]
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
    #[must_use]
    pub fn has(&self, permission: &str) -> bool {
        self.permissions.contains(permission)
    }

    /// Whether the caller owns a resource belonging to `resource_tenant`.
    ///
    /// True only when the caller has a tenant and it matches `resource_tenant`.
    /// An anonymous or tenant-less principal owns nothing — a missing tenant
    /// never grants cross-tenant access.
    #[must_use]
    pub fn owns(&self, resource_tenant: &str) -> bool {
        self.tenant.as_deref() == Some(resource_tenant)
    }
}

/// Why authentication failed. Free of any HTTP type so the trait does not drag a
/// transport into the auth contract; each transport maps the variants to its own
/// status codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthError {
    /// No bearer credential was presented. Deny by default: the zero-trust answer.
    MissingCredentials,
    /// A credential was presented but did not authenticate (bad signature,
    /// unknown key, wrong `aud`/`iss`, expired, disallowed algorithm, …).
    InvalidToken,
    /// The caller authenticated but is not permitted to do what they asked.
    Forbidden,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AuthError::MissingCredentials => "missing bearer credentials",
            AuthError::InvalidToken => "invalid bearer token",
            AuthError::Forbidden => "forbidden",
        })
    }
}

impl std::error::Error for AuthError {}

/// Turns a presented bearer token into a [`Principal`], or rejects it.
///
/// Implementations **must deny by default**: an empty or unrecognised bearer is
/// an error, never an anonymous pass. A transport calls this before it routes to
/// a handler, so an unauthenticated request never reaches business logic with a
/// forged identity.
pub trait Authenticator: Send + Sync {
    /// Authenticate `bearer` (the raw token, the `Bearer ` scheme already
    /// stripped). Return the [`Principal`] on success, or the reason it failed.
    fn authenticate(&self, bearer: &str) -> Result<Principal, AuthError>;
}

/// A static bearer-token → [`Principal`] map: the first-cut authenticator.
///
/// A fine dev/test authenticator; real OIDC/JWT is [`JwtAuthenticator`], and both
/// slot behind the same [`Authenticator`] trait. An empty map denies every
/// request — zero trust out of the box.
///
/// Tokens are indexed by their **SHA-256 digest**, never stored or compared in
/// clear: the lookup keys on the digest, so a timing side-channel over the map
/// leaks only digest bytes (which are pre-image-resistant — useless for forging a
/// token), and no bearer secret ever sits in process memory.
#[derive(Clone, Default)]
pub struct TokenAuthenticator {
    tokens: std::collections::HashMap<[u8; 32], Principal>,
}

/// The SHA-256 digest of a bearer token — the key the authenticator indexes on.
fn token_digest(token: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.finalize().into()
}

impl TokenAuthenticator {
    /// A new, empty authenticator. On its own it rejects everything.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one `token` → `principal`, builder-style. The token is stored as
    /// its SHA-256 digest, not in clear.
    #[must_use]
    pub fn with_token(mut self, token: impl Into<String>, principal: Principal) -> Self {
        self.tokens.insert(token_digest(&token.into()), principal);
        self
    }

    /// Register one `token` → `principal` in place (stored as a SHA-256 digest).
    pub fn insert(&mut self, token: impl Into<String>, principal: Principal) {
        self.tokens.insert(token_digest(&token.into()), principal);
    }

    /// How many tokens are registered. Mostly for tests and health checks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// Whether no tokens are registered (so every request is denied).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

impl Authenticator for TokenAuthenticator {
    fn authenticate(&self, bearer: &str) -> Result<Principal, AuthError> {
        if bearer.is_empty() {
            return Err(AuthError::MissingCredentials);
        }
        // Look up by the token's digest: the comparison is over digest bytes, which
        // do not reveal the token even under a timing side-channel.
        self.tokens
            .get(&token_digest(bearer))
            .cloned()
            .ok_or(AuthError::InvalidToken)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_ownership_and_permissions() {
        let p = Principal::in_tenant("alice", "acme", ["notes.read"]);
        assert!(p.has("notes.read"));
        assert!(p.owns("acme"));
        assert!(!p.owns("globex"));
        // A permission alone never implies ownership.
        assert!(!Principal::new("svc", ["notes.read"]).owns("acme"));
        assert!(!Principal::anonymous().owns("acme"));
    }

    #[test]
    fn token_authenticator_denies_by_default() {
        let auth = TokenAuthenticator::new().with_token(
            "s3cr3t",
            Principal::in_tenant("alice", "acme", ["notes.read"]),
        );
        assert_eq!(auth.authenticate("s3cr3t").unwrap().subject, "alice");
        assert_eq!(auth.authenticate("nope"), Err(AuthError::InvalidToken));
        assert_eq!(auth.authenticate(""), Err(AuthError::MissingCredentials));
        assert!(TokenAuthenticator::new().authenticate("anything").is_err());
    }
}
