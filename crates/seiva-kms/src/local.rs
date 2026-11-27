//! The KEK in a local file (feature `local`): for a deployment without Vault.
//!
//! The file holds one 32-byte key in base64 per line (blank lines and lines
//! starting with `#` are ignored). The **first** key wraps; the following ones
//! only unwrap. Rotating the KEK is: put a new key on top, restart (the keeper
//! re-wraps the ring with it), then remove the old line. In Kubernetes the file is
//! a Secret mounted read-only; the KEK exists only in the process's memory.
//!
//! A wrapped data key is `local:<kek-fingerprint>:<base64(nonce ‖ ciphertext ‖
//! tag)>`, XChaCha20-Poly1305. The fingerprint (16 hex of a SHA-256 of the key)
//! says which line wraps it, like `vault:v<N>:` names the transit key version.

use std::path::Path;

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::XChaCha20Poly1305;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::key::random_bytes;
use crate::{DataKey, GeneratedKey, KeyWrapper, KmsError};

const PREFIX: &str = "local:";
const NONCE: usize = 24;
const TAG: usize = 16;

/// A KEK read from a local file (see the module documentation).
pub struct LocalKek {
    name: String,
    /// (fingerprint, key); the first wraps.
    keys: Vec<(String, DataKey)>,
}

impl std::fmt::Debug for LocalKek {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalKek")
            .field("name", &self.name)
            .field(
                "fingerprints",
                &self.keys.iter().map(|k| &k.0).collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// The public fingerprint of a local KEK.
fn fingerprint(key: &DataKey) -> String {
    let mut h = Sha256::new();
    h.update(b"seiva-kms/local-kek\0");
    h.update(key.expose());
    hex::encode(&h.finalize()[..8])
}

fn aad(fp: &str) -> Vec<u8> {
    format!("seiva-kms/local/v1|{fp}").into_bytes()
}

impl LocalKek {
    /// Reads the key file at `path`. The KEK is named `local`
    /// ([`Self::named`] changes it).
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, KmsError> {
        let path = path.as_ref();
        let text = Zeroizing::new(std::fs::read_to_string(path).map_err(|e| {
            KmsError::Config(format!("reading the KEK file {}: {e}", path.display()))
        })?);
        Self::parse(&text, &path.display().to_string())
    }

    /// Parses the content of a key file; `source` names it in errors (which
    /// never quote a key).
    pub fn parse(text: &str, source: &str) -> Result<Self, KmsError> {
        let mut keys = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let raw = Zeroizing::new(
                STANDARD
                    .decode(line)
                    .or_else(|_| STANDARD_NO_PAD.decode(line))
                    .map_err(|_| {
                        KmsError::Config(format!("line {} of {source}: not base64", n + 1))
                    })?,
            );
            let key = DataKey::from_slice(&raw, &format!("line {} of {source}", n + 1))
                .map_err(|e| KmsError::Config(e.to_string()))?;
            keys.push(key);
        }
        Self::from_keys(keys).map_err(|e| match e {
            KmsError::Config(m) => KmsError::Config(format!("{source}: {m}")),
            other => other,
        })
    }

    /// From keys in memory: the first wraps, the rest only unwrap.
    pub fn from_keys(keys: Vec<DataKey>) -> Result<Self, KmsError> {
        if keys.is_empty() {
            return Err(KmsError::Config("no KEK (one base64 key per line)".into()));
        }
        let mut out: Vec<(String, DataKey)> = Vec::with_capacity(keys.len());
        for k in keys {
            let fp = fingerprint(&k);
            if out.iter().any(|(f, _)| *f == fp) {
                return Err(KmsError::Config(format!("KEK {fp} is listed twice")));
            }
            out.push((fp, k));
        }
        Ok(Self {
            name: "local".into(),
            keys: out,
        })
    }

    /// Names the KEK (recorded in the ring; a ring made under another name is
    /// refused). Defaults to `local`.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// The fingerprint of the key that wraps (the first line).
    pub fn current_fingerprint(&self) -> &str {
        &self.keys[0].0
    }

    fn wrap(&self, plain: &DataKey) -> Result<String, KmsError> {
        let (fp, kek) = &self.keys[0];
        let mut nonce = [0u8; NONCE];
        random_bytes(&mut nonce)?;
        let ct = XChaCha20Poly1305::new(kek.expose().into())
            .encrypt(
                (&nonce).into(),
                Payload {
                    msg: plain.expose(),
                    aad: &aad(fp),
                },
            )
            .map_err(|_| KmsError::Malformed("wrapping failed".into()))?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ct);
        Ok(format!("{PREFIX}{fp}:{}", STANDARD.encode(out)))
    }

    /// The fingerprint a wrapped key names.
    fn wrapped_fp(wrapped: &str) -> Result<(&str, &str), KmsError> {
        wrapped
            .strip_prefix(PREFIX)
            .and_then(|r| r.split_once(':'))
            .ok_or_else(|| KmsError::Malformed("not a local wrapped key".into()))
    }

    fn open(&self, wrapped: &str) -> Result<DataKey, KmsError> {
        let (fp, body) = Self::wrapped_fp(wrapped)?;
        let kek = self
            .keys
            .iter()
            .find(|(f, _)| f == fp)
            .map(|(_, k)| k)
            .ok_or_else(|| {
                KmsError::Refused(format!(
                    "the data key is wrapped by local KEK {fp}, which is not in the KEK file"
                ))
            })?;
        let raw = STANDARD
            .decode(body)
            .map_err(|_| KmsError::Malformed("local wrapped key: invalid base64".into()))?;
        if raw.len() < NONCE + TAG {
            return Err(KmsError::Malformed("local wrapped key: too short".into()));
        }
        let (nonce, ct) = raw.split_at(NONCE);
        let plain = Zeroizing::new(
            XChaCha20Poly1305::new(kek.expose().into())
                .decrypt(
                    nonce.into(),
                    Payload {
                        msg: ct,
                        aad: &aad(fp),
                    },
                )
                .map_err(|_| {
                    KmsError::Refused(format!("local KEK {fp} does not open the data key"))
                })?,
        );
        DataKey::from_slice(&plain, "local KEK")
    }
}

#[async_trait::async_trait]
impl KeyWrapper for LocalKek {
    fn provider(&self) -> &'static str {
        "local"
    }

    fn kek_id(&self) -> &str {
        &self.name
    }

    async fn generate(&self) -> Result<GeneratedKey, KmsError> {
        let plain = DataKey::random()?;
        let wrapped = self.wrap(&plain)?;
        Ok(GeneratedKey { plain, wrapped })
    }

    async fn unwrap(&self, _kek_id: &str, wrapped: &str) -> Result<DataKey, KmsError> {
        self.open(wrapped)
    }

    async fn rewrap(
        &self,
        _kek_id: &str,
        wrapped: &str,
        _plain: &DataKey,
    ) -> Result<Option<String>, KmsError> {
        let (fp, _) = Self::wrapped_fp(wrapped)?;
        if fp == self.current_fingerprint() {
            return Ok(None);
        }
        // Re-wrap what the stored form holds, not what the caller says it holds.
        let plain = self.open(wrapped)?;
        self.wrap(&plain).map(Some)
    }

    async fn health(&self) -> Result<(), KmsError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(byte: u8) -> String {
        STANDARD.encode([byte; 32])
    }

    #[test]
    fn parses_the_key_file() {
        let text = format!("# KEK\n\n{}\n  {}  \n", b64(1), b64(2));
        let k = LocalKek::parse(&text, "kek.txt").unwrap();
        assert_eq!(k.keys.len(), 2);
        assert_eq!(k.kek_id(), "local");
        assert_eq!(k.named("sentinel").kek_id(), "sentinel");
        let no_pad = STANDARD_NO_PAD.encode([3u8; 32]);
        assert!(LocalKek::parse(&no_pad, "x").is_ok());

        let short = STANDARD.encode([1u8; 16]);
        let e = LocalKek::parse(&format!("{}\n{short}\n", b64(1)), "kek.txt").unwrap_err();
        assert!(e.to_string().contains("line 2 of kek.txt"), "{e}");
        assert!(!e.to_string().contains(&short), "never quotes a key: {e}");
        assert!(LocalKek::parse("not base64!\n", "k").is_err());
        assert!(LocalKek::parse("# nothing\n\n", "k").is_err());
        assert!(LocalKek::parse(&format!("{}\n{}\n", b64(1), b64(1)), "k").is_err());
        assert!(!format!("{:?}", LocalKek::parse(&b64(1), "k").unwrap()).contains(&b64(1)));
    }

    #[test]
    fn reads_a_file() {
        let dir = std::env::temp_dir().join(format!("seiva-kms-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("kek");
        std::fs::write(&path, format!("{}\n", b64(4))).unwrap();
        assert!(LocalKek::from_file(&path).is_ok());
        assert!(matches!(
            LocalKek::from_file(dir.join("missing")),
            Err(KmsError::Config(_))
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn wraps_unwraps_and_rewraps_after_a_kek_rotation() {
        let old = LocalKek::parse(&b64(1), "k").unwrap();
        let g = old.generate().await.unwrap();
        assert!(g
            .wrapped
            .starts_with(&format!("local:{}:", old.current_fingerprint())));
        assert_eq!(old.unwrap("local", &g.wrapped).await.unwrap(), g.plain);
        assert_eq!(
            old.rewrap("local", &g.wrapped, &g.plain).await.unwrap(),
            None
        );
        assert!(old.health().await.is_ok());

        // The new key on top, the old one below: old wrappings open, and are
        // re-wrapped with the new key.
        let rotated = LocalKek::parse(&format!("{}\n{}\n", b64(2), b64(1)), "k").unwrap();
        assert_eq!(rotated.unwrap("local", &g.wrapped).await.unwrap(), g.plain);
        let re = rotated
            .rewrap("local", &g.wrapped, &g.plain)
            .await
            .unwrap()
            .expect("re-wrapped");
        assert!(re.starts_with(&format!("local:{}:", rotated.current_fingerprint())));
        assert_eq!(rotated.rewrap("local", &re, &g.plain).await.unwrap(), None);

        // Once the old line is removed, only the re-wrapped form opens.
        let only_new = LocalKek::parse(&b64(2), "k").unwrap();
        assert_eq!(only_new.unwrap("local", &re).await.unwrap(), g.plain);
        assert!(matches!(
            only_new.unwrap("local", &g.wrapped).await,
            Err(KmsError::Refused(_))
        ));
        // Tampered or foreign forms are refused.
        assert!(only_new.unwrap("local", "vault:v1:abc").await.is_err());
        let mut bad = re.clone();
        bad.pop();
        bad.push(if re.ends_with('A') { 'B' } else { 'A' });
        assert!(only_new.unwrap("local", &bad).await.is_err());
    }
}
