//! Sign and verify webhooks per the [Standard Webhooks] specification.
//!
//! The scheme is deliberately boring, which is the point: a webhook is
//! authenticated with a symmetric HMAC-SHA256 over a canonical string built
//! from the message id, the send timestamp and the raw payload. Both sides
//! hold the same secret; neither needs a public-key infrastructure.
//!
//! # The signature, exactly
//!
//! The signed content is the three fields joined with a literal dot:
//!
//! ```text
//! {id}.{timestamp}.{payload}
//! ```
//!
//! where `timestamp` is Unix time in **seconds** and `payload` is the raw
//! request body, byte for byte. The MAC is `HMAC-SHA256(secret, content)`,
//! base64-encoded (standard alphabet, padded) and emitted prefixed with its
//! scheme version:
//!
//! ```text
//! v1,<base64-signature>
//! ```
//!
//! The `webhook-signature` header carries a space-separated list of such
//! values, so a sender rotating keys can offer several signatures at once and
//! a receiver accepts the message if **any** of them verifies.
//!
//! # Headers
//!
//! An outgoing webhook carries three headers:
//!
//! - [`ID_HEADER`] (`webhook-id`) — a unique message id;
//! - [`TIMESTAMP_HEADER`] (`webhook-timestamp`) — the Unix-seconds send time;
//! - [`SIGNATURE_HEADER`] (`webhook-signature`) — the `v1,…` signature list.
//!
//! # Signing (outgoing)
//!
//! ```
//! use seiva_webhooks::Webhook;
//!
//! // The secret is base64; a `whsec_` prefix (as emitted by most providers)
//! // is stripped before decoding.
//! let wh = Webhook::new("whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw").unwrap();
//! let sig = wh.sign("msg_abc123", 1_614_265_330, br#"{"event":"ping"}"#);
//! assert!(sig.as_str().starts_with("v1,"));
//! ```
//!
//! # Verifying (incoming)
//!
//! Verification never trusts a clock carried in the payload: the claimed send
//! time comes from the `webhook-timestamp` header and "now" is supplied by the
//! caller, so the replay window is checked against a trusted clock. Injecting
//! "now" also makes the window deterministic under test.
//!
//! ```
//! use std::time::{Duration, SystemTime, UNIX_EPOCH};
//! use seiva_webhooks::{SignedHeaders, Webhook, DEFAULT_TOLERANCE};
//!
//! let wh = Webhook::new("whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw").unwrap();
//! let ts = 1_614_265_330_i64;
//! let payload = br#"{"event":"ping"}"#;
//! let sig = wh.sign("msg_abc123", ts, payload);
//!
//! let now = UNIX_EPOCH + Duration::from_secs(ts as u64);
//! let headers = SignedHeaders {
//!     id: "msg_abc123",
//!     timestamp: "1614265330",
//!     signature: sig.as_str(),
//! };
//! wh.verify(&headers, payload, now, DEFAULT_TOLERANCE).unwrap();
//! ```
//!
//! [Standard Webhooks]: https://www.standardwebhooks.com/

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// The header carrying the unique message id (`webhook-id`).
pub const ID_HEADER: &str = "webhook-id";
/// The header carrying the Unix-seconds send time (`webhook-timestamp`).
pub const TIMESTAMP_HEADER: &str = "webhook-timestamp";
/// The header carrying the `v1,…` signature list (`webhook-signature`).
pub const SIGNATURE_HEADER: &str = "webhook-signature";

/// The prefix most providers prepend to a base64 signing secret.
const SECRET_PREFIX: &str = "whsec_";

/// The default replay-protection window: a message whose timestamp is more than
/// five minutes away from "now" (in either direction) is rejected.
pub const DEFAULT_TOLERANCE: Duration = Duration::from_secs(5 * 60);

/// Everything that can go wrong signing or verifying a webhook.
///
/// Kept to small unit variants on purpose: the type stays tiny so `Result`s
/// returning it do not trip clippy's `result_large_err`, and a caller can match
/// a failure without destructuring attacker-influenced data out of the error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WebhookError {
    /// The signing secret was not valid base64 (after any `whsec_` prefix).
    #[error("invalid signing secret: not valid base64")]
    InvalidSecret,
    /// A required header (`webhook-id`, `webhook-timestamp` or
    /// `webhook-signature`) was empty.
    #[error("missing required webhook header")]
    MissingHeader,
    /// A header was present but malformed — e.g. a non-numeric timestamp or a
    /// signature list with no recognizable `v1,…` entry.
    #[error("malformed webhook header")]
    MalformedHeader,
    /// No signature in the header matched the one computed over the payload.
    #[error("no matching signature")]
    InvalidSignature,
    /// The message timestamp is older than "now" by more than the tolerance.
    #[error("message timestamp is too old")]
    TimestampTooOld,
    /// The message timestamp is further in the future than the tolerance allows.
    #[error("message timestamp is too far in the future")]
    TimestampTooNew,
}

/// The three signature-bearing headers of an incoming webhook, borrowed from
/// whatever transport delivered them.
///
/// The caller pulls the header values (from an `http::HeaderMap`, a CLI, a test
/// fixture, …) and hands them here as string slices, keeping this crate free of
/// any HTTP dependency. `signature` is the full `webhook-signature` value — the
/// possibly space-separated list of `v1,…` entries, not a single entry.
#[derive(Debug, Clone, Copy)]
pub struct SignedHeaders<'a> {
    /// The `webhook-id` value.
    pub id: &'a str,
    /// The `webhook-timestamp` value (Unix seconds, as the raw header string).
    pub timestamp: &'a str,
    /// The `webhook-signature` value (a space-separated list of `v1,…` entries).
    pub signature: &'a str,
}

/// A computed signature in its header form: `v1,<base64>`.
///
/// Produced by [`Webhook::sign`]. [`Signature::as_str`] yields the exact string
/// to place in the [`SIGNATURE_HEADER`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature(String);

impl Signature {
    /// The signature as it goes on the wire, e.g. `v1,g0hM9S…`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the signature, returning the owned header string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for Signature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A signer/verifier bound to one webhook signing secret.
///
/// Construct it from a base64 secret with [`Webhook::new`] (the common case,
/// accepting a `whsec_`-prefixed or plain base64 string) or from raw key bytes
/// with [`Webhook::from_bytes`].
#[derive(Clone)]
pub struct Webhook {
    key: Vec<u8>,
}

impl std::fmt::Debug for Webhook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the key material.
        f.debug_struct("Webhook").finish_non_exhaustive()
    }
}

impl Webhook {
    /// Build a signer from a base64 signing secret.
    ///
    /// The secret is interpreted exactly as the Standard Webhooks ecosystem
    /// does: a leading `whsec_` prefix (what most providers hand you) is
    /// stripped, and the remainder is base64-decoded (standard alphabet) to the
    /// raw HMAC key. To supply already-raw key bytes, use
    /// [`Webhook::from_bytes`] instead.
    ///
    /// # Errors
    ///
    /// Returns [`WebhookError::InvalidSecret`] if the remainder is not valid
    /// base64, or if it decodes to an **empty** key: an empty HMAC key offers no
    /// authentication (every forged message with a matching empty-key HMAC would
    /// verify), so a misconfigured empty secret is refused instead of silently
    /// accepted.
    pub fn new(secret: &str) -> Result<Self, WebhookError> {
        let b64 = secret.strip_prefix(SECRET_PREFIX).unwrap_or(secret);
        let key = STANDARD
            .decode(b64)
            .map_err(|_| WebhookError::InvalidSecret)?;
        if key.is_empty() {
            return Err(WebhookError::InvalidSecret);
        }
        Ok(Self { key })
    }

    /// Build a signer from raw key bytes, bypassing base64 decoding.
    ///
    /// Use this when the secret is held as bytes rather than as a base64 string.
    ///
    /// # Errors
    ///
    /// Returns [`WebhookError::InvalidSecret`] if `key` is empty: an empty HMAC key
    /// provides no authentication, so it is refused rather than accepted.
    pub fn from_bytes(key: &[u8]) -> Result<Self, WebhookError> {
        if key.is_empty() {
            return Err(WebhookError::InvalidSecret);
        }
        Ok(Self { key: key.to_vec() })
    }

    /// Sign a payload, returning the `v1,<base64>` value for the
    /// [`SIGNATURE_HEADER`].
    ///
    /// `id` and `timestamp` are the values that must also travel in the
    /// [`ID_HEADER`] and [`TIMESTAMP_HEADER`]; `timestamp` is Unix time in
    /// seconds. The signed content is `{id}.{timestamp}.{payload}` with the
    /// payload bytes appended verbatim.
    #[must_use]
    pub fn sign(&self, id: &str, timestamp: i64, payload: &[u8]) -> Signature {
        let mac = self.mac(id, timestamp, payload);
        let bytes = mac.finalize().into_bytes();
        Signature(format!("v1,{}", STANDARD.encode(bytes)))
    }

    /// Verify an incoming webhook against its headers and body.
    ///
    /// Two independent checks must both pass:
    ///
    /// 1. **Timestamp window.** The claimed send time (`headers.timestamp`) is
    ///    compared against `now` and must lie within `tolerance` in either
    ///    direction — a stale replay is [`WebhookError::TimestampTooOld`], a
    ///    clock-skewed or forged-future message is
    ///    [`WebhookError::TimestampTooNew`]. `now` is injected so the window is
    ///    checked against a trusted clock (and is deterministic under test);
    ///    pass `SystemTime::now()` in production.
    /// 2. **Signature.** The HMAC is recomputed over
    ///    `{id}.{timestamp}.{payload}` and compared, in constant time, against
    ///    each `v1,…` entry in `headers.signature`. The message is accepted if
    ///    any entry matches; otherwise [`WebhookError::InvalidSignature`].
    ///
    /// The timestamp used in the signed content is the raw header string, so a
    /// signature only verifies when the header the sender signed is reproduced
    /// byte for byte.
    ///
    /// # Replay within the window
    ///
    /// A valid message replayed **inside** the tolerance window verifies again —
    /// the timestamp check only bounds how far in the past/future it may be, it does
    /// not make a message single-use. A receiver that must process each event
    /// exactly once (e.g. a payment capture) **must deduplicate on the
    /// [`ID_HEADER`]** (`webhook-id`): record seen ids for at least the tolerance
    /// window and drop a repeat. This crate verifies authenticity and freshness; at-
    /// most-once delivery is the caller's, by id.
    ///
    /// # Errors
    ///
    /// - [`WebhookError::MissingHeader`] if any of the three headers is empty.
    /// - [`WebhookError::MalformedHeader`] if the timestamp is not an integer
    ///   or the signature list has no recognizable `v1,…` entry.
    /// - [`WebhookError::TimestampTooOld`] / [`WebhookError::TimestampTooNew`]
    ///   if the timestamp is outside the window.
    /// - [`WebhookError::InvalidSignature`] if no signature matched.
    pub fn verify(
        &self,
        headers: &SignedHeaders<'_>,
        payload: &[u8],
        now: SystemTime,
        tolerance: Duration,
    ) -> Result<(), WebhookError> {
        if headers.id.is_empty() || headers.timestamp.is_empty() || headers.signature.is_empty() {
            return Err(WebhookError::MissingHeader);
        }

        let ts: i64 = headers
            .timestamp
            .parse()
            .map_err(|_| WebhookError::MalformedHeader)?;
        check_timestamp(ts, now, tolerance)?;

        // Recompute the MAC once over the canonical content.
        let expected = self.mac(headers.id, ts, payload).finalize().into_bytes();

        let mut saw_candidate = false;
        for entry in headers.signature.split_whitespace() {
            // Each entry is `<version>,<base64>`; we only speak v1.
            let Some(b64) = entry.strip_prefix("v1,") else {
                continue;
            };
            let Ok(candidate) = STANDARD.decode(b64) else {
                continue;
            };
            saw_candidate = true;
            // `ct_eq` of the equal-length byte strings runs in constant time,
            // so a near-miss cannot be distinguished from a far-miss by timing.
            if candidate.len() == expected.len() && ct_eq(&candidate, &expected) {
                return Ok(());
            }
        }

        if saw_candidate {
            Err(WebhookError::InvalidSignature)
        } else {
            // No `v1,…` entry was present at all: the header is malformed rather
            // than merely carrying a wrong signature.
            Err(WebhookError::MalformedHeader)
        }
    }

    /// Build the keyed MAC over the canonical `{id}.{timestamp}.{payload}`
    /// content, left unfinalized so the caller decides whether to `finalize`
    /// (signing) or finalize-and-compare (verifying).
    fn mac(&self, id: &str, timestamp: i64, payload: &[u8]) -> HmacSha256 {
        // HMAC accepts a key of any length, so `new_from_slice` never errors.
        let mut mac =
            HmacSha256::new_from_slice(&self.key).expect("HMAC-SHA256 accepts a key of any length");
        mac.update(id.as_bytes());
        mac.update(b".");
        mac.update(timestamp.to_string().as_bytes());
        mac.update(b".");
        mac.update(payload);
        mac
    }
}

/// Reject a timestamp that falls outside `[now - tolerance, now + tolerance]`.
fn check_timestamp(ts: i64, now: SystemTime, tolerance: Duration) -> Result<(), WebhookError> {
    let now_secs = unix_secs(now);
    let tol = tolerance.as_secs() as i64;
    // `saturating_sub` keeps a pathological far-past/far-future timestamp from
    // overflowing the comparison.
    if now_secs.saturating_sub(ts) > tol {
        return Err(WebhookError::TimestampTooOld);
    }
    if ts.saturating_sub(now_secs) > tol {
        return Err(WebhookError::TimestampTooNew);
    }
    Ok(())
}

/// "now" as signed Unix seconds. Times before the epoch read as negative.
fn unix_secs(now: SystemTime) -> i64 {
    match now.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// Constant-time equality of two equal-length byte strings: the comparison
/// time depends only on the length, never on where the first difference is.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    debug_assert_eq!(a.len(), b.len());
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET_B64: &str = "MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";
    const ID: &str = "msg_2KWPBgLlAfxdpx2AI54pPJ85f4W";
    const TS: i64 = 1_614_265_330;
    const PAYLOAD: &[u8] = br#"{"event":"ping","data":{"id":1}}"#;

    fn at(ts: i64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(ts as u64)
    }

    fn headers<'a>(id: &'a str, ts: &'a str, sig: &'a str) -> SignedHeaders<'a> {
        SignedHeaders {
            id,
            timestamp: ts,
            signature: sig,
        }
    }

    #[test]
    fn whsec_prefixed_and_plain_base64_decode_to_the_same_key() {
        let prefixed = Webhook::new(&format!("whsec_{SECRET_B64}")).unwrap();
        let plain = Webhook::new(SECRET_B64).unwrap();
        assert_eq!(prefixed.key, plain.key);
        // And the key is the raw decode of the base64 secret.
        assert_eq!(plain.key, STANDARD.decode(SECRET_B64).unwrap());
    }

    #[test]
    fn invalid_base64_secret_is_rejected() {
        // `!` is outside the base64 alphabet. Compare the error (the `Ok`
        // variant holds a `Webhook`, which is intentionally not `PartialEq`).
        assert_eq!(
            Webhook::new("not valid !!!").err(),
            Some(WebhookError::InvalidSecret)
        );
    }

    #[test]
    fn sign_emits_the_v1_scheme_prefix() {
        let wh = Webhook::new(SECRET_B64).unwrap();
        let sig = wh.sign(ID, TS, PAYLOAD);
        assert!(sig.as_str().starts_with("v1,"));
        // The part after the prefix is valid standard base64.
        assert!(STANDARD.decode(&sig.as_str()[3..]).is_ok());
    }

    #[test]
    fn sign_then_verify_roundtrip_succeeds() {
        let wh = Webhook::new(SECRET_B64).unwrap();
        let sig = wh.sign(ID, TS, PAYLOAD);
        let h = headers(ID, "1614265330", sig.as_str());
        assert_eq!(wh.verify(&h, PAYLOAD, at(TS), DEFAULT_TOLERANCE), Ok(()));
    }

    #[test]
    fn from_bytes_roundtrips_with_a_raw_key() {
        let wh = Webhook::from_bytes(b"a raw secret key, any length").unwrap();
        let sig = wh.sign(ID, TS, PAYLOAD);
        let h = headers(ID, "1614265330", sig.as_str());
        assert_eq!(wh.verify(&h, PAYLOAD, at(TS), DEFAULT_TOLERANCE), Ok(()));
    }

    #[test]
    fn an_empty_secret_is_refused() {
        // Neither a base64 nor a raw empty key is accepted: an empty HMAC key
        // authenticates nothing.
        assert!(matches!(Webhook::new(""), Err(WebhookError::InvalidSecret)));
        assert!(matches!(
            Webhook::from_bytes(b""),
            Err(WebhookError::InvalidSecret)
        ));
    }

    #[test]
    fn a_tampered_payload_fails() {
        let wh = Webhook::new(SECRET_B64).unwrap();
        let sig = wh.sign(ID, TS, PAYLOAD);
        let h = headers(ID, "1614265330", sig.as_str());
        let tampered = br#"{"event":"ping","data":{"id":2}}"#;
        assert_eq!(
            wh.verify(&h, tampered, at(TS), DEFAULT_TOLERANCE),
            Err(WebhookError::InvalidSignature)
        );
    }

    #[test]
    fn a_wrong_secret_fails() {
        let signer = Webhook::new(SECRET_B64).unwrap();
        let sig = signer.sign(ID, TS, PAYLOAD);
        let other = Webhook::new("dGhlIHdyb25nIHNlY3JldCBlbnRpcmVseQ==").unwrap();
        let h = headers(ID, "1614265330", sig.as_str());
        assert_eq!(
            other.verify(&h, PAYLOAD, at(TS), DEFAULT_TOLERANCE),
            Err(WebhookError::InvalidSignature)
        );
    }

    #[test]
    fn a_timestamp_too_far_in_the_past_is_rejected() {
        let wh = Webhook::new(SECRET_B64).unwrap();
        let sig = wh.sign(ID, TS, PAYLOAD);
        let h = headers(ID, "1614265330", sig.as_str());
        // "now" is six minutes after the claimed send time; window is five.
        let now = at(TS + 6 * 60);
        assert_eq!(
            wh.verify(&h, PAYLOAD, now, DEFAULT_TOLERANCE),
            Err(WebhookError::TimestampTooOld)
        );
    }

    #[test]
    fn a_timestamp_too_far_in_the_future_is_rejected() {
        let wh = Webhook::new(SECRET_B64).unwrap();
        let sig = wh.sign(ID, TS, PAYLOAD);
        let h = headers(ID, "1614265330", sig.as_str());
        // "now" is six minutes before the claimed send time.
        let now = at(TS - 6 * 60);
        assert_eq!(
            wh.verify(&h, PAYLOAD, now, DEFAULT_TOLERANCE),
            Err(WebhookError::TimestampTooNew)
        );
    }

    #[test]
    fn the_window_edge_is_inclusive() {
        let wh = Webhook::new(SECRET_B64).unwrap();
        let sig = wh.sign(ID, TS, PAYLOAD);
        let h = headers(ID, "1614265330", sig.as_str());
        // Exactly five minutes late: still inside the window.
        assert_eq!(
            wh.verify(&h, PAYLOAD, at(TS + 5 * 60), DEFAULT_TOLERANCE),
            Ok(())
        );
        assert_eq!(
            wh.verify(&h, PAYLOAD, at(TS - 5 * 60), DEFAULT_TOLERANCE),
            Ok(())
        );
    }

    #[test]
    fn missing_headers_are_reported() {
        let wh = Webhook::new(SECRET_B64).unwrap();
        let h = headers("", "1614265330", "v1,abc");
        assert_eq!(
            wh.verify(&h, PAYLOAD, at(TS), DEFAULT_TOLERANCE),
            Err(WebhookError::MissingHeader)
        );
    }

    #[test]
    fn a_non_numeric_timestamp_is_malformed() {
        let wh = Webhook::new(SECRET_B64).unwrap();
        let h = headers(ID, "not-a-number", "v1,abc");
        assert_eq!(
            wh.verify(&h, PAYLOAD, at(TS), DEFAULT_TOLERANCE),
            Err(WebhookError::MalformedHeader)
        );
    }

    #[test]
    fn a_signature_list_with_no_v1_entry_is_malformed() {
        let wh = Webhook::new(SECRET_B64).unwrap();
        // Only an unknown future scheme version is offered.
        let h = headers(ID, "1614265330", "v2,c29tZXRoaW5n");
        assert_eq!(
            wh.verify(&h, PAYLOAD, at(TS), DEFAULT_TOLERANCE),
            Err(WebhookError::MalformedHeader)
        );
    }

    #[test]
    fn multiple_signatures_accept_if_any_matches() {
        let wh = Webhook::new(SECRET_B64).unwrap();
        let good = wh.sign(ID, TS, PAYLOAD);
        // A bogus v1 entry first, then the valid one, space-separated.
        let list = format!("v1,AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHHIIIIJJJJKKA= {good}");
        let h = headers(ID, "1614265330", &list);
        assert_eq!(wh.verify(&h, PAYLOAD, at(TS), DEFAULT_TOLERANCE), Ok(()));
    }
}
