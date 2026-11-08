//! Seiva's authentication / zero-trust layer for the MCP HTTP transport.
//!
//! This is the part the brief assigns to the framework: "HTTP needs the
//! authentication and zero trust of the framework". It is **owned here**, not by
//! `dorm-mcp` — the crate intentionally does not use DORM's OAuth. The contract
//! is deliberately small for this first cut: an [`Authenticator`] turns a bearer
//! token into a [`Principal`] (a subject, the tenant it is scoped to, and whether
//! it may write), and **denies by default** — no token, or an unknown token, is a
//! rejection, never a silent pass.
//!
//! The reference implementation is [`TokenAuthenticator`], a static token →
//! principal map. That is a fine first cut; real OIDC / mTLS / OAuth
//! token-exchange is a later step and slots in behind the same trait without the
//! HTTP layer changing — the endpoint only ever sees an `Authenticator`.

use std::collections::HashMap;

use dorm_mcp::HttpToken;

/// The authenticated caller: who they are, the tenant their requests are forced
/// to, and whether they may issue writes.
///
/// The tenant here is authoritative — the HTTP layer copies it onto the
/// per-request [`dorm_mcp::ServerConfig`], so a tenant-scoped query is pinned to
/// the principal's tenant and can never be widened by a tool argument. `tenant`
/// of `None` means the principal is not tenant-scoped (a tenant-wide or
/// single-tenant deployment).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    /// A stable, loggable identity for the caller. Never the bearer token.
    pub subject: String,
    /// The tenant every tenant-scoped query is constrained to, if any.
    pub tenant: Option<String>,
    /// Whether this principal is permitted to write (still gated by the server
    /// being started writable — see the HTTP layer).
    pub can_write: bool,
}

/// Why authentication failed. Kept free of any HTTP type so the trait does not
/// drag the transport into the auth contract; the HTTP layer maps each variant
/// to a status code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthError {
    /// No bearer credential was presented. Zero trust: the default answer.
    MissingCredentials,
    /// A credential was presented but did not authenticate.
    InvalidToken,
    /// The caller authenticated but is not permitted to do what they asked.
    Forbidden,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            AuthError::MissingCredentials => "missing bearer credentials",
            AuthError::InvalidToken => "invalid bearer token",
            AuthError::Forbidden => "forbidden",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for AuthError {}

/// Turns a presented bearer token into a [`Principal`], or rejects it.
///
/// Implementations **must deny by default**: an empty or unrecognised bearer is
/// an error, never an anonymous pass. The HTTP endpoint calls this before it
/// reads the request body or touches DORM, so an unauthenticated request never
/// reaches the protocol core.
pub trait Authenticator: Send + Sync {
    /// Authenticate `bearer` (the raw token, with the `Bearer ` scheme already
    /// stripped). Return the [`Principal`] on success, or the reason it failed.
    fn authenticate(&self, bearer: &str) -> Result<Principal, AuthError>;
}

/// A static bearer-token → [`Principal`] map: the first-cut authenticator.
///
/// Built either directly ([`with_token`](TokenAuthenticator::with_token)) or
/// bridged from DORM's own token spec via
/// [`from_http_tokens`](TokenAuthenticator::from_http_tokens) /
/// [`dorm_mcp::parse_tokens`], so an existing `DELONIX_ORM_MCP_TOKENS`-style
/// configuration can be reused while the authentication *decision* stays Seiva's.
/// An empty map denies every request — zero trust out of the box.
///
/// Tokens are indexed by their **SHA-256 digest**, never stored or compared in
/// clear: a timing side-channel over the lookup leaks only digest bytes
/// (pre-image-resistant), and no bearer secret sits in process memory.
#[derive(Clone, Default)]
pub struct TokenAuthenticator {
    tokens: HashMap<[u8; 32], Principal>,
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
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one `token` → `principal`, builder-style (stored as a SHA-256 digest).
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
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// Whether no tokens are registered (so every request is denied).
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Bridge DORM's [`HttpToken`] list into Seiva principals, so a deployment
    /// already configured with DORM's `token=tenant:rw` spec (parsed by
    /// [`dorm_mcp::parse_tokens`]) can be served here. The principal's `subject`
    /// is derived from its tenant (or `anonymous`) — never from the secret token,
    /// which stays only as the map key.
    pub fn from_http_tokens(tokens: &[HttpToken]) -> Self {
        let mut auth = Self::new();
        for t in tokens {
            let subject = t.tenant.clone().unwrap_or_else(|| "anonymous".to_string());
            auth.insert(
                t.token.clone(),
                Principal {
                    subject,
                    tenant: t.tenant.clone(),
                    can_write: t.can_write,
                },
            );
        }
        auth
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

    fn principal(tenant: Option<&str>, can_write: bool) -> Principal {
        Principal {
            subject: tenant.unwrap_or("anonymous").to_string(),
            tenant: tenant.map(str::to_string),
            can_write,
        }
    }

    #[test]
    fn empty_authenticator_denies_everything() {
        let auth = TokenAuthenticator::new();
        assert!(auth.is_empty());
        assert_eq!(auth.authenticate(""), Err(AuthError::MissingCredentials));
        assert_eq!(auth.authenticate("anything"), Err(AuthError::InvalidToken));
    }

    #[test]
    fn a_known_token_authenticates_to_its_principal() {
        let auth = TokenAuthenticator::new().with_token("s3cr3t", principal(Some("acme"), true));
        let p = auth.authenticate("s3cr3t").expect("known token");
        assert_eq!(p.tenant.as_deref(), Some("acme"));
        assert!(p.can_write);
        assert_eq!(p.subject, "acme");
    }

    #[test]
    fn a_missing_token_is_distinct_from_an_invalid_one() {
        let auth = TokenAuthenticator::new().with_token("ok", principal(None, false));
        assert_eq!(auth.authenticate(""), Err(AuthError::MissingCredentials));
        assert_eq!(auth.authenticate("nope"), Err(AuthError::InvalidToken));
        assert!(auth.authenticate("ok").is_ok());
    }

    #[test]
    fn bridges_from_dorm_http_tokens() {
        let tokens = dorm_mcp::parse_tokens("alpha=acme:rw, beta=:ro");
        let auth = TokenAuthenticator::from_http_tokens(&tokens);
        assert_eq!(auth.len(), 2);

        let a = auth.authenticate("alpha").expect("alpha");
        assert_eq!(a.tenant.as_deref(), Some("acme"));
        assert!(a.can_write);
        assert_eq!(a.subject, "acme");

        let b = auth.authenticate("beta").expect("beta");
        assert_eq!(b.tenant, None);
        assert!(!b.can_write);
        assert_eq!(b.subject, "anonymous");
    }
}
