//! HTTP-facing building blocks for durable idempotency (ADR 0011).
//!
//! The durable guarantee itself lives in the **application layer**, inside the
//! `UnitOfWork` transaction, so the reservation commits atomically with the
//! domain mutation and the outbox write (`seiva_ports::IdempotencyStore`, proven
//! against a real database by `seiva_data_dorm::DormIdempotencyStore`). A generic
//! outer middleware could not keep that atomicity — it runs above any
//! transaction — so this module gives the transport only its half:
//!
//! 1. [`IdempotencyKey`] — validate the `Idempotency-Key` header (bounded length,
//!    visible-ASCII only), via the [`OptionalIdempotencyKey`] extractor.
//! 2. [`scoped_key`] — fold the caller's tenant + subject + the operation
//!    (method + path, which carries the API version segment) into the key the
//!    store sees, so two tenants (or two identities) can never collide or replay
//!    one another's result.
//! 3. [`fingerprint`] — a canonical SHA-256 of the request's semantically
//!    relevant arguments (method, path, query, and the body as canonical JSON —
//!    `serde_json`'s default sorts object keys). The store answers
//!    [`Mismatch`](seiva_ports::IdempotencyOutcome::Mismatch) when the same key
//!    arrives with a different fingerprint.
//! 4. [`mismatch_problem`] / [`in_progress_problem`] — map the non-`Fresh`
//!    outcomes to RFC 9457 responses.
//!
//! The application threads the key + fingerprint into `IdempotencyStore::begin`
//! inside its transaction; on `Fresh` it performs the effect and `finish`es, on
//! `Replay` it returns the stored result, on `Mismatch`/`InProgress` it returns
//! the problem below. Re-authorization holds because the scoped key is bound to
//! the caller's identity: a different caller gets a different key and never reads
//! a prior caller's response.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use seiva_identity::Principal;
use sha2::{Digest, Sha256};

use crate::problem::Problem;

/// The header carrying the client's idempotency key.
pub const IDEMPOTENCY_HEADER: &str = "idempotency-key";

/// The maximum accepted key length (bytes). Longer keys are rejected `400`, so a
/// client cannot store unbounded data under a key.
pub const MAX_KEY_LEN: usize = 255;

/// A validated `Idempotency-Key`: non-empty, at most [`MAX_KEY_LEN`] bytes, and
/// visible ASCII only (`0x21..=0x7E`) so it cannot smuggle control characters or
/// whitespace into a header or a storage key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Validates `raw` into a key, or a `400` [`Problem`] explaining why not.
    ///
    /// # Errors
    ///
    /// Returns a `400` problem when the key is empty, too long, or contains a
    /// byte outside visible ASCII.
    pub fn parse(raw: &str) -> Result<Self, Problem> {
        if raw.is_empty() {
            return Err(bad_key("the Idempotency-Key must not be empty"));
        }
        if raw.len() > MAX_KEY_LEN {
            return Err(bad_key("the Idempotency-Key is too long (max 255 bytes)"));
        }
        if !raw.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
            return Err(bad_key(
                "the Idempotency-Key must be visible ASCII with no spaces or control characters",
            ));
        }
        Ok(Self(raw.to_owned()))
    }

    /// The validated key as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn bad_key(detail: &str) -> Problem {
    Problem::new(StatusCode::BAD_REQUEST, "Bad Idempotency-Key").with_detail(detail)
}

/// Extractor for an **optional** idempotency key: `Ok(None)` when the header is
/// absent (idempotency is opt-in per request, never forced on every `POST`),
/// `Err(400)` when it is present but malformed.
#[derive(Debug, Clone)]
pub struct OptionalIdempotencyKey(pub Option<IdempotencyKey>);

impl<S: Sync> FromRequestParts<S> for OptionalIdempotencyKey {
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        match parts.headers.get(IDEMPOTENCY_HEADER) {
            None => Ok(Self(None)),
            Some(value) => {
                let raw = value
                    .to_str()
                    .map_err(|_| bad_key("the Idempotency-Key must be valid ASCII"))?;
                Ok(Self(Some(IdempotencyKey::parse(raw)?)))
            }
        }
    }
}

/// Folds the caller's identity and the operation into the key the store sees, so
/// keys are isolated per tenant and per subject (and per method+path, which
/// carries the API version). Returns a fixed-length hex digest: bounded, and
/// free of any delimiter a component could spoof.
#[must_use]
pub fn scoped_key(principal: &Principal, method: &str, path: &str, key: &IdempotencyKey) -> String {
    let mut hasher = Sha256::new();
    for part in [
        principal.tenant.as_deref().unwrap_or(""),
        principal.subject.as_str(),
        method,
        path,
        key.as_str(),
    ] {
        // Length-prefix each field so distinct tuples cannot alias.
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    hex(&hasher.finalize())
}

/// A canonical SHA-256 fingerprint of the request's semantically relevant
/// arguments. `body` is serialised with `serde_json` (whose default `Map` is a
/// `BTreeMap`, so object keys are sorted) — the same body reorders to the same
/// bytes, so a retry with logically-equal content matches.
#[must_use]
pub fn fingerprint(
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &serde_json::Value,
) -> String {
    let mut hasher = Sha256::new();
    for part in [
        method.as_bytes(),
        path.as_bytes(),
        query.unwrap_or("").as_bytes(),
    ] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    let body_bytes = serde_json::to_vec(body).unwrap_or_default();
    hasher.update((body_bytes.len() as u64).to_be_bytes());
    hasher.update(&body_bytes);
    hex(&hasher.finalize())
}

/// The `409` problem for a key reused with a **different** request fingerprint:
/// the same key must mean the same operation, so the server does not execute.
#[must_use]
pub fn mismatch_problem() -> Problem {
    Problem::new(StatusCode::CONFLICT, "Idempotency-Key reused")
        .with_detail("this Idempotency-Key was already used for a request with different content")
}

/// The `409` problem for a key whose first request is still in flight: the caller
/// retries after a short delay (it adds its own `Retry-After` if it wishes).
#[must_use]
pub fn in_progress_problem() -> Problem {
    Problem::new(StatusCode::CONFLICT, "Idempotent request in progress")
        .with_detail("a request with this Idempotency-Key is still being processed; retry shortly")
}

/// Lowercase hex of a byte slice.
fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_validation_bounds_and_charset() {
        assert!(IdempotencyKey::parse("abc-123_XYZ").is_ok());
        assert!(IdempotencyKey::parse("").is_err(), "empty rejected");
        assert!(
            IdempotencyKey::parse(&"x".repeat(MAX_KEY_LEN + 1)).is_err(),
            "over-long rejected"
        );
        assert!(
            IdempotencyKey::parse("has space").is_err(),
            "space rejected"
        );
        assert!(
            IdempotencyKey::parse("ctl\u{7}").is_err(),
            "control rejected"
        );
    }

    #[test]
    fn scoped_key_isolates_tenant_subject_and_operation() {
        let key = IdempotencyKey::parse("k1").unwrap();
        let acme = Principal::in_tenant("alice", "acme", ["x"]);
        let globex = Principal::in_tenant("alice", "globex", ["x"]);
        let bob = Principal::in_tenant("bob", "acme", ["x"]);

        let base = scoped_key(&acme, "POST", "/v1/orders", &key);
        // Same inputs → same key.
        assert_eq!(base, scoped_key(&acme, "POST", "/v1/orders", &key));
        // Different tenant, subject, method or path → different key.
        assert_ne!(base, scoped_key(&globex, "POST", "/v1/orders", &key));
        assert_ne!(base, scoped_key(&bob, "POST", "/v1/orders", &key));
        assert_ne!(base, scoped_key(&acme, "PUT", "/v1/orders", &key));
        assert_ne!(base, scoped_key(&acme, "POST", "/v1/invoices", &key));
    }

    #[test]
    fn fingerprint_is_canonical_and_content_sensitive() {
        // Object key order does not matter (canonical JSON).
        let a = fingerprint("POST", "/v1/o", None, &json!({"a":1,"b":2}));
        let b = fingerprint("POST", "/v1/o", None, &json!({"b":2,"a":1}));
        assert_eq!(a, b, "logically-equal bodies fingerprint the same");
        // Different content → different fingerprint.
        assert_ne!(a, fingerprint("POST", "/v1/o", None, &json!({"a":1,"b":3})));
        // Query participates.
        assert_ne!(
            a,
            fingerprint("POST", "/v1/o", Some("x=1"), &json!({"a":1,"b":2}))
        );
    }

    #[test]
    fn non_fresh_outcomes_map_to_409() {
        assert_eq!(mismatch_problem().status, StatusCode::CONFLICT);
        assert_eq!(in_progress_problem().status, StatusCode::CONFLICT);
    }
}
