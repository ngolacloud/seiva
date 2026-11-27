//! Gateway keys: what a client presents instead of a provider key.
//!
//! A key is issued by the organisation, bound to a principal (subject,
//! permissions, tenant) and an area, and stored **only as its SHA-256 digest** —
//! the configuration never holds a key in clear, and the lookup compares digests.
//! A provider key is never a caller credential (ADR 0048 §2).

use std::collections::HashMap;

use seiva_identity::{AuthError, Authenticator, Principal};
use sha2::{Digest, Sha256};

/// One key, as configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEntry {
    /// The SHA-256 of the key, 64 hex characters.
    pub sha256: String,
    /// Who the key acts as.
    pub principal: Principal,
    /// The caller's area, for residency and budget.
    pub area: Option<String>,
}

/// The configured keys: an [`Authenticator`] over digests, plus each key's area.
#[derive(Clone, Default)]
pub struct GatewayKeys {
    by_digest: HashMap<[u8; 32], (Principal, Option<String>)>,
}

impl std::fmt::Debug for GatewayKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayKeys")
            .field("keys", &self.by_digest.len())
            .finish()
    }
}

/// Why a key entry was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("key for `{subject}`: {reason}")]
pub struct KeyError {
    /// The entry's subject.
    pub subject: String,
    /// What is wrong.
    pub reason: String,
}

impl GatewayKeys {
    /// No keys: every request is refused.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an entry. The digest must be 64 hex characters and the subject not
    /// empty; a digest used twice is refused.
    pub fn add(&mut self, entry: KeyEntry) -> Result<(), KeyError> {
        let err = |reason: &str| KeyError {
            subject: entry.principal.subject.clone(),
            reason: reason.to_owned(),
        };
        if entry.principal.subject.trim().is_empty() {
            return Err(err("the subject is empty"));
        }
        let digest =
            parse_hex32(&entry.sha256).ok_or_else(|| err("sha256 must be 64 hex characters"))?;
        if self.by_digest.contains_key(&digest) {
            return Err(err("the same key digest is configured twice"));
        }
        self.by_digest.insert(digest, (entry.principal, entry.area));
        Ok(())
    }

    /// Add an entry, builder-style.
    pub fn with(mut self, entry: KeyEntry) -> Result<Self, KeyError> {
        self.add(entry)?;
        Ok(self)
    }

    /// How many keys are configured.
    pub fn len(&self) -> usize {
        self.by_digest.len()
    }

    /// Whether no key is configured.
    pub fn is_empty(&self) -> bool {
        self.by_digest.is_empty()
    }

    /// The principal and area of `key`.
    pub fn lookup(&self, key: &str) -> Result<(Principal, Option<String>), AuthError> {
        if key.is_empty() {
            return Err(AuthError::MissingCredentials);
        }
        self.by_digest
            .get(&digest(key))
            .cloned()
            .ok_or(AuthError::InvalidToken)
    }
}

impl Authenticator for GatewayKeys {
    fn authenticate(&self, bearer: &str) -> Result<Principal, AuthError> {
        self.lookup(bearer).map(|(p, _)| p)
    }
}

/// The SHA-256 of `key`, as 64 lower-case hex characters — what goes in the
/// configuration.
pub fn hash_key(key: &str) -> String {
    digest(key).iter().map(|b| format!("{b:02x}")).collect()
}

fn digest(key: &str) -> [u8; 32] {
    Sha256::digest(key.as_bytes()).into()
}

fn parse_hex32(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let pair = std::str::from_utf8(chunk).ok()?;
        out[i] = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str) -> KeyEntry {
        KeyEntry {
            sha256: hash_key(key),
            principal: Principal::new("claude-code-ana", ["llm.code"]),
            area: Some("engineering".into()),
        }
    }

    #[test]
    fn a_key_authenticates_by_its_digest_only() {
        let keys = GatewayKeys::new()
            .with(entry("sk-gw-ana-0123456789"))
            .unwrap();
        let (p, area) = keys.lookup("sk-gw-ana-0123456789").unwrap();
        assert_eq!(p.subject, "claude-code-ana");
        assert_eq!(area.as_deref(), Some("engineering"));
        assert_eq!(
            keys.authenticate("sk-gw-ana-wrong"),
            Err(AuthError::InvalidToken)
        );
        assert_eq!(keys.authenticate(""), Err(AuthError::MissingCredentials));
        assert!(!format!("{keys:?}").contains("sk-gw"));
    }

    #[test]
    fn bad_entries_are_refused() {
        let mut keys = GatewayKeys::new();
        let mut e = entry("k");
        e.sha256 = "abc".into();
        assert!(keys.add(e).is_err());
        keys.add(entry("k")).unwrap();
        assert!(keys.add(entry("k")).is_err(), "the same digest twice");
        let mut anon = entry("other");
        anon.principal = Principal::anonymous();
        assert!(keys.add(anon).is_err());
    }

    #[test]
    fn the_digest_is_the_standard_sha256() {
        assert_eq!(
            hash_key("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
