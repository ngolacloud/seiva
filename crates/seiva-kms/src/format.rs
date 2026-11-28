//! The sealed formats and the associated data.
//!
//! - **`sv1:<key-id>:<base64(nonce ‖ ciphertext ‖ tag)>`** — XChaCha20-Poly1305
//!   with a random 24-byte nonce and the caller's associated data. The only
//!   format this crate seals.
//! - **`kms1:<key-id>:<base64(nonce ‖ ciphertext ‖ tag)>`** — dxstorage's
//!   AES-256-GCM with a 12-byte nonce and *no* associated data. Opened only
//!   through the explicit [`KeyRing::open_kms1`](crate::KeyRing::open_kms1), so
//!   a caller never accepts an unbound blob where a bound one is expected.
//!
//! Base64 is the standard alphabet with padding, as dxstorage writes it.

use aes_gcm::Aes256Gcm;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::XChaCha20Poly1305;
use zeroize::Zeroizing;

use crate::key::random_bytes;
use crate::{DataKey, KmsError};

/// The prefix of the format this crate seals.
pub const SV1_PREFIX: &str = "sv1:";
/// The prefix of dxstorage's format (opened, never sealed).
pub const KMS1_PREFIX: &str = "kms1:";

const XNONCE: usize = 24;
const GCM_NONCE: usize = 12;
const TAG: usize = 16;

/// Which format a sealed string is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealedFormat {
    /// `sv1:` — XChaCha20-Poly1305 with associated data.
    Sv1,
    /// `kms1:` — dxstorage's AES-256-GCM without associated data.
    Kms1,
}

impl SealedFormat {
    /// The format and key id of `sealed`, or `None` when it is neither (a
    /// legacy blob of the application's own, or not sealed at all).
    pub fn of(sealed: &str) -> Option<(SealedFormat, &str)> {
        let (format, rest) = if let Some(r) = sealed.strip_prefix(SV1_PREFIX) {
            (SealedFormat::Sv1, r)
        } else {
            (SealedFormat::Kms1, sealed.strip_prefix(KMS1_PREFIX)?)
        };
        let (id, _) = rest.split_once(':')?;
        valid_key_id(id).then_some((format, id))
    }
}

/// A key id: 1–128 of `[A-Za-z0-9._-]` (never `:`, the format's separator).
pub fn valid_key_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The associated data a secret is bound to: the blob opens only with the same
/// bytes. Copied to another row, account or organisation, it does not open.
///
/// [`Aad::new`] builds the shape ADR 0049 recommends,
/// `<org>|<purpose>|<record>` (e.g. `acme|identity.totp|user-42`).
#[derive(Clone, PartialEq, Eq)]
pub struct Aad(Vec<u8>);

impl Aad {
    /// `<org>|<purpose>|<record>`. `org` and `purpose` must be non-empty and
    /// free of `|` (so the split is unambiguous); `record` must be non-empty and
    /// may contain anything.
    pub fn new(org: &str, purpose: &str, record: &str) -> Result<Self, KmsError> {
        for (name, v) in [("organisation", org), ("purpose", purpose)] {
            if v.is_empty() || v.contains('|') {
                return Err(KmsError::Config(format!(
                    "associated data: the {name} must be non-empty and contain no '|'"
                )));
            }
        }
        if record.is_empty() {
            return Err(KmsError::Config(
                "associated data: the record must be non-empty".into(),
            ));
        }
        Ok(Self(format!("{org}|{purpose}|{record}").into_bytes()))
    }

    /// Any non-empty bytes, for a caller with its own binding scheme.
    pub fn raw(bytes: impl Into<Vec<u8>>) -> Result<Self, KmsError> {
        let b = bytes.into();
        if b.is_empty() {
            return Err(KmsError::Config(
                "associated data must not be empty: a secret is always bound to its context".into(),
            ));
        }
        Ok(Self(b))
    }

    /// The bytes given to the cipher.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for Aad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Aad({:?})", String::from_utf8_lossy(&self.0))
    }
}

/// `sv1:<id>:<base64>` of `plain` under `key`, bound to `aad`.
pub(crate) fn seal_sv1(
    id: &str,
    key: &DataKey,
    aad: &Aad,
    plain: &[u8],
) -> Result<String, KmsError> {
    let cipher = XChaCha20Poly1305::new(key.expose().into());
    let mut nonce = [0u8; XNONCE];
    random_bytes(&mut nonce)?;
    let ct = cipher
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: plain,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| KmsError::BadSealed("plaintext too long".into()))?;
    let mut out = Vec::with_capacity(XNONCE + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(format!("{SV1_PREFIX}{id}:{}", STANDARD.encode(out)))
}

/// The id and the decoded body of `sealed` in `format`, checked for length.
pub(crate) fn split(sealed: &str, format: SealedFormat) -> Result<(&str, Vec<u8>), KmsError> {
    let (prefix, min) = match format {
        SealedFormat::Sv1 => (SV1_PREFIX, XNONCE + TAG),
        SealedFormat::Kms1 => (KMS1_PREFIX, GCM_NONCE + TAG),
    };
    let rest = sealed.strip_prefix(prefix).ok_or_else(|| {
        let found = match SealedFormat::of(sealed) {
            Some((f, _)) => format!("{f:?}"),
            None => "an unknown format".into(),
        };
        KmsError::BadSealed(format!("expected {prefix} but found {found}"))
    })?;
    let (id, body) = rest
        .split_once(':')
        .ok_or_else(|| KmsError::BadSealed("no key id".into()))?;
    if !valid_key_id(id) {
        return Err(KmsError::BadSealed("invalid key id".into()));
    }
    let raw = STANDARD
        .decode(body)
        .map_err(|_| KmsError::BadSealed("invalid base64".into()))?;
    if raw.len() < min {
        return Err(KmsError::BadSealed("too short".into()));
    }
    Ok((id, raw))
}

pub(crate) fn open_sv1(
    key: &DataKey,
    aad: &Aad,
    raw: &[u8],
) -> Result<Zeroizing<Vec<u8>>, KmsError> {
    let (nonce, ct) = raw.split_at(XNONCE);
    XChaCha20Poly1305::new(key.expose().into())
        .decrypt(
            nonce.into(),
            Payload {
                msg: ct,
                aad: aad.as_bytes(),
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| KmsError::Decrypt)
}

pub(crate) fn open_kms1(key: &DataKey, raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, KmsError> {
    let (nonce, ct) = raw.split_at(GCM_NONCE);
    Aes256Gcm::new(key.expose().into())
        .decrypt(nonce.into(), ct)
        .map(Zeroizing::new)
        .map_err(|_| KmsError::Decrypt)
}

#[cfg(test)]
pub(crate) fn seal_kms1_for_test(id: &str, key: &DataKey, plain: &[u8]) -> String {
    // Exactly dxstorage's `seal_with`: base64(nonce12 ‖ AES-256-GCM(plain)).
    let mut nonce = [0u8; GCM_NONCE];
    random_bytes(&mut nonce).unwrap();
    let ct = Aes256Gcm::new(key.expose().into())
        .encrypt((&nonce).into(), plain)
        .unwrap();
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    format!("{KMS1_PREFIX}{id}:{}", STANDARD.encode(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aad_shape() {
        let a = Aad::new("acme", "identity.totp", "user|42").unwrap();
        assert_eq!(a.as_bytes(), b"acme|identity.totp|user|42");
        assert!(Aad::new("a|b", "p", "r").is_err());
        assert!(Aad::new("a", "", "r").is_err());
        assert!(Aad::new("a", "p", "").is_err());
        assert!(Aad::raw(Vec::new()).is_err());
        assert_eq!(Aad::raw("x").unwrap().as_bytes(), b"x");
    }

    #[test]
    fn formats_and_ids() {
        assert_eq!(
            SealedFormat::of("sv1:k-1:AAAA"),
            Some((SealedFormat::Sv1, "k-1"))
        );
        assert_eq!(
            SealedFormat::of("kms1:vault-abc:AAAA"),
            Some((SealedFormat::Kms1, "vault-abc"))
        );
        assert_eq!(SealedFormat::of("legacy-base64"), None);
        assert_eq!(SealedFormat::of("sv1:no-body"), None);
        assert_eq!(SealedFormat::of("sv1:bad id:AAAA"), None);
        assert!(valid_key_id("local-0123abcd.v2_x"));
        assert!(!valid_key_id(""));
        assert!(!valid_key_id("a:b"));
        assert!(!valid_key_id(&"a".repeat(129)));
    }
}
