//! Seiva's authentication boundary for the MCP HTTP transport.
//!
//! The identity model is the framework's **single** one: [`Principal`],
//! [`Authenticator`], [`AuthError`] and [`TokenAuthenticator`] are re-exported
//! from [`seiva_identity`] — the very types `seiva-http`'s middleware
//! authenticates against (ADR 0006). There is no MCP-private `Principal` any
//! more: a token minted for the HTTP edge means exactly the same thing here,
//! and a future OIDC/JWT or `X-API-Key` authenticator slots in behind the same
//! trait without this transport changing.
//!
//! This module adds only the glue the transport-free, DORM-free identity crate
//! cannot carry:
//!
//! * [`WRITE_PERMISSION`] / [`can_write`] — the convention that maps a
//!   principal's permission set to DORM's per-request write bit. The shared
//!   [`Principal`] carries *permissions*, not a `can_write` flag; MCP reads one
//!   well-known permission out of that set.
//! * [`from_http_tokens`] — bridges DORM's `token=tenant:rw` spec (parsed by
//!   [`dorm_mcp::parse_tokens`]) into a [`TokenAuthenticator`], so a deployment
//!   already configured with a `DELONIX_ORM_MCP_TOKENS`-style list can be served
//!   here while the authentication *decision* stays Seiva's. This is DORM-coupled
//!   glue, which is why it lives here and not in `seiva-identity`.

use dorm_mcp::HttpToken;

pub use seiva_identity::{AuthError, Authenticator, Principal, TokenAuthenticator};

/// The permission a [`Principal`] must hold for its MCP requests to be allowed to
/// write. The shared [`Principal`] has no `can_write` field — it carries a
/// permission *set* — so the MCP transport expresses "may write" as the presence
/// of this one well-known permission, checked by [`can_write`].
///
/// It is still gated by the server being started writable (see the HTTP layer):
/// holding the permission is necessary, never sufficient.
pub const WRITE_PERMISSION: &str = "mcp.write";

/// Whether `principal` is permitted to issue writes: it holds
/// [`WRITE_PERMISSION`]. The HTTP layer narrows a request to
/// `server_writable && can_write(principal)`.
#[must_use]
pub fn can_write(principal: &Principal) -> bool {
    principal.has(WRITE_PERMISSION)
}

/// Bridge DORM's [`HttpToken`] list into a [`TokenAuthenticator`], so a
/// deployment already configured with DORM's `token=tenant:rw` spec (parsed by
/// [`dorm_mcp::parse_tokens`]) can be served here.
///
/// Each token's `can_write` bit is translated into the [`WRITE_PERMISSION`]
/// permission on the resulting [`Principal`]; the principal's `subject` is derived
/// from its tenant (or `anonymous`) — never from the secret token, which stays
/// only as the map key (a SHA-256 digest inside [`TokenAuthenticator`]).
#[must_use]
pub fn from_http_tokens(tokens: &[HttpToken]) -> TokenAuthenticator {
    let mut auth = TokenAuthenticator::new();
    for t in tokens {
        let subject = t.tenant.clone().unwrap_or_else(|| "anonymous".to_string());
        // `can_write` → the one write permission; no permissions means read-only.
        let permissions = t.can_write.then(|| WRITE_PERMISSION.to_string());
        let principal = match &t.tenant {
            Some(tenant) => Principal::in_tenant(subject, tenant.clone(), permissions),
            None => Principal::new(subject, permissions),
        };
        auth.insert(t.token.clone(), principal);
    }
    auth
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_permission_drives_can_write() {
        let writer = Principal::in_tenant("acme", "acme", [WRITE_PERMISSION]);
        let reader = Principal::in_tenant("acme", "acme", Vec::<String>::new());
        assert!(can_write(&writer));
        assert!(!can_write(&reader));
        assert!(!can_write(&Principal::anonymous()));
    }

    #[test]
    fn a_known_token_authenticates_to_its_principal() {
        let auth = TokenAuthenticator::new().with_token(
            "s3cr3t",
            Principal::in_tenant("acme", "acme", [WRITE_PERMISSION]),
        );
        let p = auth.authenticate("s3cr3t").expect("known token");
        assert_eq!(p.tenant.as_deref(), Some("acme"));
        assert!(can_write(&p));
        assert_eq!(p.subject, "acme");
    }

    #[test]
    fn empty_authenticator_denies_everything() {
        let auth = TokenAuthenticator::new();
        assert!(auth.is_empty());
        assert_eq!(auth.authenticate(""), Err(AuthError::MissingCredentials));
        assert_eq!(auth.authenticate("anything"), Err(AuthError::InvalidToken));
    }

    #[test]
    fn bridges_from_dorm_http_tokens() {
        let tokens = dorm_mcp::parse_tokens("alpha=acme:rw, beta=:ro");
        let auth = from_http_tokens(&tokens);
        assert_eq!(auth.len(), 2);

        let a = auth.authenticate("alpha").expect("alpha");
        assert_eq!(a.tenant.as_deref(), Some("acme"));
        assert!(can_write(&a));
        assert_eq!(a.subject, "acme");

        let b = auth.authenticate("beta").expect("beta");
        assert_eq!(b.tenant, None);
        assert!(!can_write(&b));
        assert_eq!(b.subject, "anonymous");
    }
}
