//! Authenticate a connection **before** the upgrade completes, and only from an
//! allowed `Origin`.
//!
//! This is the deny-by-default security seam of ADR 0031. A transport MUST call
//! [`authorize_upgrade`] and complete the WebSocket/SSE upgrade only on `Ok`:
//!
//! 1. the request's `Origin` must be in an explicit [`OriginAllowlist`] (stops
//!    cross-site WebSocket hijacking, CSWSH — a browser always sends `Origin`);
//! 2. a non-empty **connection-token** (short-lived, minted by a normal
//!    authenticated REST call — never a long-lived session cookie riding the
//!    upgrade) must authenticate through a [`RealtimeAuth`], yielding the
//!    [`Principal`] + [`Scope`] the socket is bound to for its whole life.
//!
//! A missing token, an unknown token, or a disallowed origin all deny the
//! upgrade, so an unauthenticated peer never reaches the hub.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use seiva_identity::Principal;
use seiva_ports::Scope;

use crate::RealtimeError;

/// The verified identity a connection is bound to for its lifetime: who the peer
/// is and the isolation scope its frames run under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authenticated {
    /// The authenticated caller (ADR 0006).
    pub principal: Principal,
    /// The isolation scope (tenant or global) the connection is pinned to.
    pub scope: Scope,
}

impl Authenticated {
    /// Bind a connection to `principal` under `scope`.
    #[must_use]
    pub fn new(principal: Principal, scope: Scope) -> Self {
        Self { principal, scope }
    }
}

/// Validates a connection-token into an [`Authenticated`] identity, or rejects it.
///
/// Implementations **must deny by default**: an empty or unrecognised token is a
/// rejection ([`RealtimeError::Unauthenticated`]), never an anonymous pass. This
/// is the real-time twin of `seiva_identity::Authenticator`, async because token
/// validation may hit a store or a JWKS.
#[async_trait]
pub trait RealtimeAuth: Send + Sync {
    /// Authenticate the raw connection-token presented at the upgrade.
    ///
    /// # Errors
    ///
    /// [`RealtimeError::Unauthenticated`] for a missing/empty/unknown token.
    async fn authenticate(&self, token: &str) -> Result<Authenticated, RealtimeError>;
}

/// An explicit allowlist of request `Origin`s permitted to open a connection.
///
/// Deny-by-default: a freshly [`new`](Self::new) allowlist permits nothing. A
/// browser always sends `Origin`, so a *missing* origin is also denied unless
/// the deployment deliberately opts out with [`any`](Self::any) (dev/non-browser
/// clients). This is the CSWSH guard of ADR 0031.
#[derive(Clone, Debug, Default)]
pub struct OriginAllowlist {
    allowed: HashSet<String>,
    any: bool,
}

impl OriginAllowlist {
    /// An empty allowlist that denies every origin (including a missing one).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An allowlist that permits **any** origin, including a missing one.
    ///
    /// An explicit, visible opt-out of the CSWSH guard — for local development or
    /// trusted non-browser clients only, never a browser-facing deployment.
    #[must_use]
    pub fn any() -> Self {
        Self {
            allowed: HashSet::new(),
            any: true,
        }
    }

    /// Add one permitted origin (e.g. `https://app.example.com`), builder-style.
    #[must_use]
    pub fn with(mut self, origin: impl Into<String>) -> Self {
        self.allowed.insert(origin.into());
        self
    }

    /// Add one permitted origin in place.
    pub fn insert(&mut self, origin: impl Into<String>) {
        self.allowed.insert(origin.into());
    }

    /// Whether `origin` (the request's `Origin` header, if any) is permitted.
    #[must_use]
    pub fn allows(&self, origin: Option<&str>) -> bool {
        if self.any {
            return true;
        }
        match origin {
            Some(o) => self.allowed.contains(o),
            None => false,
        }
    }
}

/// The deny-by-default upgrade gate: run this **before** completing the upgrade.
///
/// Checks the `Origin` against `allowlist` first (cheap, stops CSWSH), then
/// authenticates the connection-token through `auth`. Returns the identity the
/// socket binds to, or the reason the upgrade is refused.
///
/// # Errors
///
/// - [`RealtimeError::ForbiddenOrigin`] if `origin` is not allowed;
/// - [`RealtimeError::Unauthenticated`] if the token is missing/empty/invalid.
pub async fn authorize_upgrade(
    auth: &dyn RealtimeAuth,
    token: Option<&str>,
    origin: Option<&str>,
    allowlist: &OriginAllowlist,
) -> Result<Authenticated, RealtimeError> {
    // Origin first: a cross-site upgrade is refused before any token work.
    if !allowlist.allows(origin) {
        return Err(RealtimeError::ForbiddenOrigin);
    }
    let token = token
        .filter(|t| !t.is_empty())
        .ok_or(RealtimeError::Unauthenticated)?;
    auth.authenticate(token).await
}

/// The SHA-256 digest of a connection-token — the key the dev authenticator
/// indexes on, so no bearer secret sits in process memory in clear.
fn token_digest(token: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.finalize().into()
}

/// A static connection-token → [`Authenticated`] map: the dev/test authenticator.
///
/// The real-time counterpart to `seiva_identity::TokenAuthenticator`. Tokens are
/// stored only as their SHA-256 digest (hash at rest); an empty map denies every
/// upgrade — zero trust out of the box. Real deployments validate a short-lived
/// signed token (minted by a REST call) with their own [`RealtimeAuth`].
#[derive(Clone, Default)]
pub struct StaticRealtimeAuth {
    tokens: HashMap<[u8; 32], Authenticated>,
}

impl StaticRealtimeAuth {
    /// A new, empty authenticator. On its own it rejects every upgrade.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one `token` → `identity`, builder-style (stored as a digest).
    #[must_use]
    pub fn with_token(mut self, token: impl AsRef<str>, identity: Authenticated) -> Self {
        self.tokens.insert(token_digest(token.as_ref()), identity);
        self
    }

    /// Register one `token` → `identity` in place (stored as a digest).
    pub fn insert(&mut self, token: impl AsRef<str>, identity: Authenticated) {
        self.tokens.insert(token_digest(token.as_ref()), identity);
    }

    /// Whether no tokens are registered (so every upgrade is denied).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// How many tokens are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tokens.len()
    }
}

#[async_trait]
impl RealtimeAuth for StaticRealtimeAuth {
    async fn authenticate(&self, token: &str) -> Result<Authenticated, RealtimeError> {
        if token.is_empty() {
            return Err(RealtimeError::Unauthenticated);
        }
        self.tokens
            .get(&token_digest(token))
            .cloned()
            .ok_or(RealtimeError::Unauthenticated)
    }
}
