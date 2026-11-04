//! An [`Authenticator`] that verifies an OIDC/JWT bearer against a JWKS.
//!
//! It verifies with the audited [`jsonwebtoken`] crate (on `ring`): the signing
//! algorithm is an allowlist of **RS256 and ES256 only** — never a shared-secret
//! `HS*`, never `alg: none` — and `iss`, `aud`, `exp` and `nbf` are all checked.
//! A token that fails any step yields no [`Principal`] (ADR 0006).
//!
//! Key material is **injected**, not fetched here: the authenticator holds the
//! [`JwkSet`] by value, so this stays network-free and
//! deterministically testable. Fetching/rotating the JWKS from the issuer's
//! `jwks_uri` (and the issuer allowlist that guards against SSRF) belongs to the
//! wiring layer, not here.

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde_json::Value;

use crate::{AuthError, Authenticator, Principal};

/// The default claim carrying the caller's tenant.
pub const DEFAULT_TENANT_CLAIM: &str = "tenant";
/// The default claim carrying the caller's permissions (a JSON array of strings).
/// When absent, a space-delimited `scope` claim is used instead.
pub const DEFAULT_PERMISSION_CLAIM: &str = "permissions";

/// The signing algorithms Seiva will verify — asymmetric only. A token header
/// naming anything else (a shared-secret `HS*`, or `none`) is refused before the
/// signature is checked, closing the algorithm-confusion hole.
const ALLOWED_ALGORITHMS: [Algorithm; 2] = [Algorithm::RS256, Algorithm::ES256];

/// Verifies an OIDC/JWT bearer against an injected JWKS and maps its claims to a
/// [`Principal`].
///
/// Build it from the keys and the expected issuer/audience, then (optionally)
/// point it at the claims that carry the tenant and permissions:
///
/// ```no_run
/// use jsonwebtoken::jwk::JwkSet;
/// use seiva_identity::JwtAuthenticator;
///
/// # fn demo(jwks: JwkSet) {
/// let auth = JwtAuthenticator::new(jwks, "https://id.example.com/", "seiva-api")
///     .with_tenant_claim("org_id");
/// # let _ = auth;
/// # }
/// ```
pub struct JwtAuthenticator {
    jwks: JwkSet,
    validation: Validation,
    tenant_claim: String,
    permission_claim: String,
}

impl JwtAuthenticator {
    /// A verifier for tokens minted by `issuer` for `audience`, trusting the keys
    /// in `jwks`. Algorithms are fixed to RS256 and ES256; `exp` and `nbf` are
    /// validated.
    #[must_use]
    pub fn new(jwks: JwkSet, issuer: impl Into<String>, audience: impl Into<String>) -> Self {
        let mut validation = Validation::new(Algorithm::RS256);
        // A placeholder; the real algorithm is pinned per request to the (allowlisted)
        // one named in the token header — jsonwebtoken refuses a `from_jwk` key against
        // a multi-family algorithm list, so the single-alg pin both works and is tighter.
        validation.algorithms = ALLOWED_ALGORITHMS.to_vec();
        validation.set_issuer(&[issuer.into()]);
        validation.set_audience(&[audience.into()]);
        validation.validate_nbf = true;
        Self {
            jwks,
            validation,
            tenant_claim: DEFAULT_TENANT_CLAIM.to_string(),
            permission_claim: DEFAULT_PERMISSION_CLAIM.to_string(),
        }
    }

    /// Read the tenant from a different claim (default [`DEFAULT_TENANT_CLAIM`]).
    #[must_use]
    pub fn with_tenant_claim(mut self, claim: impl Into<String>) -> Self {
        self.tenant_claim = claim.into();
        self
    }

    /// Read the permissions from a different claim (default
    /// [`DEFAULT_PERMISSION_CLAIM`]); a space-delimited `scope` is the fallback.
    #[must_use]
    pub fn with_permission_claim(mut self, claim: impl Into<String>) -> Self {
        self.permission_claim = claim.into();
        self
    }

    fn permissions_from(&self, claims: &Value) -> std::collections::BTreeSet<String> {
        if let Some(array) = claims.get(&self.permission_claim).and_then(Value::as_array) {
            return array
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
        }
        // OIDC convention: a space-delimited `scope` string.
        if let Some(scope) = claims.get("scope").and_then(Value::as_str) {
            return scope.split_whitespace().map(str::to_string).collect();
        }
        std::collections::BTreeSet::new()
    }
}

impl Authenticator for JwtAuthenticator {
    fn authenticate(&self, bearer: &str) -> Result<Principal, AuthError> {
        if bearer.is_empty() {
            return Err(AuthError::MissingCredentials);
        }
        // The header names the key (`kid`) and the algorithm; both are checked
        // against the allowlist and the JWKS before the signature is trusted.
        let header = decode_header(bearer).map_err(|_| AuthError::InvalidToken)?;
        if !ALLOWED_ALGORITHMS.contains(&header.alg) {
            return Err(AuthError::InvalidToken);
        }
        let kid = header.kid.ok_or(AuthError::InvalidToken)?;
        let jwk = self.jwks.find(&kid).ok_or(AuthError::InvalidToken)?;
        let key = DecodingKey::from_jwk(jwk).map_err(|_| AuthError::InvalidToken)?;

        // Pin verification to the single (allowlisted) algorithm the header names.
        // A mismatch between it and the JWK's key family (RS/ES confusion) makes
        // `decode` fail here, so the pin is a tightening, not a trust of the header.
        let mut validation = self.validation.clone();
        validation.algorithms = vec![header.alg];
        let data =
            decode::<Value>(bearer, &key, &validation).map_err(|_| AuthError::InvalidToken)?;
        let claims = data.claims;

        let subject = claims
            .get("sub")
            .and_then(Value::as_str)
            .ok_or(AuthError::InvalidToken)?
            .to_string();
        let tenant = claims
            .get(&self.tenant_claim)
            .and_then(Value::as_str)
            .map(str::to_string);
        let permissions = self.permissions_from(&claims);

        Ok(Principal {
            subject,
            tenant,
            permissions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, get_current_timestamp, EncodingKey, Header};
    use serde_json::json;

    const PRIVATE_KEY_PEM: &[u8] = include_bytes!("../tests/fixtures/private_key.pem");
    const JWKS_JSON: &str = include_str!("../tests/fixtures/jwks.json");

    const ISSUER: &str = "https://id.example.com/";
    const AUDIENCE: &str = "seiva-api";

    fn jwks() -> JwkSet {
        serde_json::from_str(JWKS_JSON).expect("the fixture JWKS parses")
    }

    fn authenticator() -> JwtAuthenticator {
        JwtAuthenticator::new(jwks(), ISSUER, AUDIENCE)
    }

    /// Signs `claims` with the fixture RSA key under the matching `kid`.
    fn sign(alg: Algorithm, kid: Option<&str>, claims: &Value) -> String {
        let mut header = Header::new(alg);
        header.kid = kid.map(str::to_string);
        let key = EncodingKey::from_rsa_pem(PRIVATE_KEY_PEM).expect("fixture key loads");
        encode(&header, claims, &key).expect("signing succeeds")
    }

    fn valid_claims() -> Value {
        json!({
            "sub": "alice",
            "iss": ISSUER,
            "aud": AUDIENCE,
            "exp": get_current_timestamp() + 3600,
            "tenant": "acme",
            "permissions": ["notes.read", "notes.create"],
        })
    }

    #[test]
    fn a_valid_token_maps_claims_to_a_principal() {
        let token = sign(Algorithm::RS256, Some("test-key-1"), &valid_claims());
        let p = authenticator().authenticate(&token).expect("valid token");
        assert_eq!(p.subject, "alice");
        assert_eq!(p.tenant.as_deref(), Some("acme"));
        assert!(p.has("notes.read") && p.has("notes.create"));
    }

    #[test]
    fn a_tampered_signature_is_rejected() {
        let token = sign(Algorithm::RS256, Some("test-key-1"), &valid_claims());
        // Flip the last character of the signature segment.
        let mut bytes = token.into_bytes();
        let last = bytes.last_mut().unwrap();
        *last = if *last == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap();
        assert_eq!(
            authenticator().authenticate(&tampered),
            Err(AuthError::InvalidToken)
        );
    }

    #[test]
    fn the_wrong_audience_is_rejected() {
        let mut claims = valid_claims();
        claims["aud"] = json!("someone-else");
        let token = sign(Algorithm::RS256, Some("test-key-1"), &claims);
        assert_eq!(
            authenticator().authenticate(&token),
            Err(AuthError::InvalidToken)
        );
    }

    #[test]
    fn the_wrong_issuer_is_rejected() {
        let mut claims = valid_claims();
        claims["iss"] = json!("https://evil.example/");
        let token = sign(Algorithm::RS256, Some("test-key-1"), &claims);
        assert_eq!(
            authenticator().authenticate(&token),
            Err(AuthError::InvalidToken)
        );
    }

    #[test]
    fn an_expired_token_is_rejected() {
        let mut claims = valid_claims();
        // Well past the default 60s leeway, so it is unambiguously expired.
        claims["exp"] = json!(get_current_timestamp() - 3600);
        let token = sign(Algorithm::RS256, Some("test-key-1"), &claims);
        assert_eq!(
            authenticator().authenticate(&token),
            Err(AuthError::InvalidToken)
        );
    }

    #[test]
    fn an_unknown_key_id_is_rejected() {
        let token = sign(Algorithm::RS256, Some("some-other-kid"), &valid_claims());
        assert_eq!(
            authenticator().authenticate(&token),
            Err(AuthError::InvalidToken)
        );
    }

    #[test]
    fn a_disallowed_algorithm_is_rejected() {
        // An HS256 token (shared-secret) must be refused: the allowlist is
        // RS256/ES256, so algorithm confusion never reaches signature checking.
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some("test-key-1".to_string());
        let token = encode(
            &header,
            &valid_claims(),
            &EncodingKey::from_secret(b"attacker-chosen-secret"),
        )
        .unwrap();
        assert_eq!(
            authenticator().authenticate(&token),
            Err(AuthError::InvalidToken)
        );
    }

    #[test]
    fn an_empty_bearer_is_missing_credentials() {
        assert_eq!(
            authenticator().authenticate(""),
            Err(AuthError::MissingCredentials)
        );
    }

    #[test]
    fn permissions_fall_back_to_the_scope_claim() {
        let mut claims = valid_claims();
        claims.as_object_mut().unwrap().remove("permissions");
        claims["scope"] = json!("notes.read notes.delete");
        let token = sign(Algorithm::RS256, Some("test-key-1"), &claims);
        let p = authenticator().authenticate(&token).expect("valid");
        assert!(p.has("notes.read") && p.has("notes.delete"));
    }
}
