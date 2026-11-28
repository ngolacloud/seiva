//! The key ring: the wrapped form that is stored, and the unwrapped form that
//! seals and opens. Pure: no I/O, no clock (times are passed in).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::format::{self, valid_key_id, Aad, SealedFormat};
use crate::{DataKey, KmsError};

/// The version of the stored ring's JSON shape this crate writes and reads.
pub const RING_FORMAT: u32 = 1;

/// A data key as stored: the guardian's wrapped form of it, never the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedKey {
    /// The id written in sealed blobs (`sv1:<id>:`).
    pub id: String,
    /// The guardian that wrapped it (`local`, `vault`).
    pub provider: String,
    /// The KEK that wrapped it, as the guardian names it
    /// ([`KeyWrapper::kek_id`](crate::KeyWrapper::kek_id)).
    pub kek: String,
    /// The guardian's wrapped form (`vault:v1:…`, `local:<kek-fp>:…`).
    pub wrapped: String,
    /// When the data key was made (Unix seconds).
    pub created: i64,
}

impl WrappedKey {
    /// A new entry for a data key fresh from `provider`/`kek`: its id is
    /// `<provider>-<12 hex of SHA-256(wrapped)>`, as dxstorage names them.
    pub fn new(provider: &str, kek: &str, wrapped: String, created: i64) -> Self {
        let digest = hex::encode(Sha256::digest(wrapped.as_bytes()));
        Self {
            id: format!("{provider}-{}", &digest[..12]),
            provider: provider.to_string(),
            kek: kek.to_string(),
            wrapped,
            created,
        }
    }
}

/// The stored key ring: the current data key and the earlier ones, each
/// wrapped. This is what a [`RingStore`](crate::RingStore) keeps (as JSON,
/// [`WrappedRing::to_json`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedRing {
    /// The shape's version ([`RING_FORMAT`]).
    pub format: u32,
    /// Seals new secrets (and opens).
    pub current: WrappedKey,
    /// Earlier keys: they only open what they sealed, until it is re-sealed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous: Vec<WrappedKey>,
    /// When the current key replaced the previous ones (Unix seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotated_at: Option<i64>,
}

impl WrappedRing {
    /// A ring of one key.
    pub fn new(current: WrappedKey) -> Self {
        Self {
            format: RING_FORMAT,
            current,
            previous: Vec::new(),
            rotated_at: None,
        }
    }

    /// The current key, then the previous ones.
    pub fn keys(&self) -> impl Iterator<Item = &WrappedKey> {
        std::iter::once(&self.current).chain(self.previous.iter())
    }

    /// Checks the ring is one this crate can use: the known format, valid and
    /// unique key ids, nothing empty. Fails closed.
    pub fn validate(&self) -> Result<(), KmsError> {
        if self.format != RING_FORMAT {
            return Err(KmsError::Ring(format!(
                "format {} is not supported (this build reads format {RING_FORMAT})",
                self.format
            )));
        }
        let mut seen = std::collections::HashSet::new();
        for k in self.keys() {
            if !valid_key_id(&k.id) {
                return Err(KmsError::Ring(format!("invalid key id {:?}", k.id)));
            }
            if !seen.insert(k.id.as_str()) {
                return Err(KmsError::Ring(format!("key id {} appears twice", k.id)));
            }
            if k.provider.is_empty() || k.kek.is_empty() || k.wrapped.is_empty() {
                return Err(KmsError::Ring(format!("key {} is incomplete", k.id)));
            }
        }
        Ok(())
    }

    /// The JSON a store keeps.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a ring always serialises")
    }

    /// Parses and [validates](Self::validate) a stored ring.
    pub fn from_json(json: &str) -> Result<Self, KmsError> {
        let ring: Self =
            serde_json::from_str(json).map_err(|e| KmsError::Ring(format!("unreadable: {e}")))?;
        ring.validate()?;
        Ok(ring)
    }

    /// Reads dxstorage's stored ring (`kms/dek`, the `DekRecord` of F8/F11: the
    /// current key's fields at the top level, `vault_key` for the KEK, `kms`
    /// absent for Vault), so dxstorage can adopt this crate without re-wrapping.
    /// Its key ids are kept, so its `kms1:` blobs keep opening.
    pub fn from_dxstorage_json(json: &str) -> Result<Self, KmsError> {
        #[derive(Deserialize)]
        struct DxKey {
            id: String,
            vault_key: String,
            #[serde(default)]
            kms: Option<String>,
            ciphertext: String,
            created: i64,
        }
        #[derive(Deserialize)]
        struct DxRing {
            #[serde(flatten)]
            current: DxKey,
            #[serde(default)]
            previous: Vec<DxKey>,
            #[serde(default)]
            rotated: Option<i64>,
        }
        let dx: DxRing = serde_json::from_str(json)
            .map_err(|e| KmsError::Ring(format!("unreadable dxstorage ring: {e}")))?;
        let key = |k: DxKey| WrappedKey {
            id: k.id,
            provider: k.kms.unwrap_or_else(|| "vault".into()),
            kek: k.vault_key,
            wrapped: k.ciphertext,
            created: k.created,
        };
        let ring = Self {
            format: RING_FORMAT,
            current: key(dx.current),
            previous: dx.previous.into_iter().map(key).collect(),
            rotated_at: dx.rotated,
        };
        ring.validate()?;
        Ok(ring)
    }
}

/// The unwrapped key ring: seals with the current key and opens with any key it
/// holds. Built by [`RingKeeper`](crate::RingKeeper) (or by hand with
/// [`KeyRing::from_parts`]) after the guardian unwrapped every key.
///
/// Cloning is cheap enough (a few 32-byte keys); every copy wipes its keys when
/// dropped. `Debug` prints key ids only.
#[derive(Clone)]
pub struct KeyRing {
    wrapped: WrappedRing,
    /// One per key of `wrapped`, same order (current first).
    keys: Vec<(String, DataKey)>,
    /// Keys outside the stored ring that only open (e.g. dxstorage's static
    /// keys while it moves to a guardian).
    open_only: Vec<(String, DataKey)>,
}

impl std::fmt::Debug for KeyRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyRing")
            .field("current", &self.current_id())
            .field("previous", &self.previous_ids())
            .field(
                "open_only",
                &self.open_only.iter().map(|k| &k.0).collect::<Vec<_>>(),
            )
            .field("rotated_at", &self.wrapped.rotated_at)
            .finish()
    }
}

impl KeyRing {
    /// The ring `wrapped` with its unwrapped `keys`. Every key of the ring must
    /// be given exactly once, and nothing else: a ring with a key missing is
    /// refused, never used partially.
    pub fn from_parts(
        wrapped: WrappedRing,
        keys: impl IntoIterator<Item = (String, DataKey)>,
    ) -> Result<Self, KmsError> {
        wrapped.validate()?;
        let mut given: Vec<(String, DataKey)> = keys.into_iter().collect();
        let mut ordered = Vec::with_capacity(given.len());
        for w in wrapped.keys() {
            let pos = given
                .iter()
                .position(|(id, _)| *id == w.id)
                .ok_or_else(|| KmsError::Ring(format!("data key {} is not unwrapped", w.id)))?;
            ordered.push(given.swap_remove(pos));
        }
        if let Some((id, _)) = given.first() {
            return Err(KmsError::Ring(format!(
                "data key {id} is not in the ring (or given twice)"
            )));
        }
        Ok(Self {
            wrapped,
            keys: ordered,
            open_only: Vec::new(),
        })
    }

    /// Adds a key that only opens (never seals, never stored with the ring).
    pub fn with_open_only(mut self, id: &str, key: DataKey) -> Result<Self, KmsError> {
        if !valid_key_id(id) {
            return Err(KmsError::Config(format!("invalid key id {id:?}")));
        }
        if self.holds(id) {
            return Err(KmsError::Config(format!(
                "key id {id} is already in the ring"
            )));
        }
        self.open_only.push((id.to_string(), key));
        Ok(self)
    }

    /// The stored form of this ring.
    pub fn wrapped(&self) -> &WrappedRing {
        &self.wrapped
    }

    /// The key that seals.
    pub fn current_id(&self) -> &str {
        &self.wrapped.current.id
    }

    /// The earlier keys still in the ring (newest first).
    pub fn previous_ids(&self) -> Vec<&str> {
        self.wrapped
            .previous
            .iter()
            .map(|k| k.id.as_str())
            .collect()
    }

    /// When the current key replaced the previous ones.
    pub fn rotated_at(&self) -> Option<i64> {
        self.wrapped.rotated_at
    }

    /// The ring holds key `id` (stored or open-only).
    pub fn holds(&self, id: &str) -> bool {
        self.key(id).is_some()
    }

    pub(crate) fn key(&self, id: &str) -> Option<&DataKey> {
        self.keys
            .iter()
            .chain(self.open_only.iter())
            .find(|(kid, _)| kid == id)
            .map(|(_, k)| k)
    }

    /// Seals `plain` with the current key, bound to `aad`:
    /// `sv1:<current-id>:<base64(nonce ‖ ciphertext ‖ tag)>`.
    pub fn seal(&self, aad: &Aad, plain: &[u8]) -> Result<String, KmsError> {
        let (id, key) = &self.keys[0];
        format::seal_sv1(id, key, aad, plain)
    }

    /// Opens an `sv1:` blob sealed by any key of the ring, with the same `aad`
    /// it was sealed with. A `kms1:` blob is refused here: it carries no
    /// associated data, so it is opened only by [`Self::open_kms1`].
    pub fn open(&self, aad: &Aad, sealed: &str) -> Result<Zeroizing<Vec<u8>>, KmsError> {
        let (id, raw) = format::split(sealed, SealedFormat::Sv1)?;
        let key = self
            .key(id)
            .ok_or_else(|| KmsError::UnknownKey(id.to_string()))?;
        format::open_sv1(key, aad, &raw)
    }

    /// Opens dxstorage's `kms1:` blob (AES-256-GCM, no associated data) with the
    /// key it names. For migrating: re-seal what this returns with
    /// [`Self::seal`] and the right [`Aad`].
    pub fn open_kms1(&self, sealed: &str) -> Result<Zeroizing<Vec<u8>>, KmsError> {
        let (id, raw) = format::split(sealed, SealedFormat::Kms1)?;
        let key = self
            .key(id)
            .ok_or_else(|| KmsError::UnknownKey(id.to_string()))?;
        format::open_kms1(key, &raw)
    }

    /// `sealed` should be re-sealed: it is not an `sv1:` blob of the current key.
    pub fn needs_reseal(&self, sealed: &str) -> bool {
        SealedFormat::of(sealed) != Some((SealedFormat::Sv1, self.current_id()))
    }

    /// The ring after a rotation: `next` (with its unwrapped `key`) seals, the
    /// current key joins the previous ones. `now` is the rotation time.
    pub fn rotated(&self, next: WrappedKey, key: DataKey, now: i64) -> Result<Self, KmsError> {
        if self.holds(&next.id) {
            return Err(KmsError::Ring(format!(
                "the new data key {} is already in the ring",
                next.id
            )));
        }
        let mut previous = vec![self.wrapped.current.clone()];
        previous.extend(self.wrapped.previous.iter().cloned());
        let wrapped = WrappedRing {
            format: RING_FORMAT,
            current: next,
            previous,
            rotated_at: Some(now),
        };
        wrapped.validate()?;
        let mut keys = vec![(wrapped.current.id.clone(), key)];
        keys.extend(self.keys.iter().cloned());
        Ok(Self {
            wrapped,
            keys,
            open_only: self.open_only.clone(),
        })
    }

    /// The ring without its previous keys (after everything they sealed was
    /// re-sealed).
    pub fn retired(&self) -> Self {
        Self {
            wrapped: WrappedRing {
                format: RING_FORMAT,
                current: self.wrapped.current.clone(),
                previous: Vec::new(),
                rotated_at: None,
            },
            keys: vec![self.keys[0].clone()],
            open_only: self.open_only.clone(),
        }
    }

    /// The same ring with key `id` stored in a new wrapped form (the guardian
    /// re-wrapped it with a newer KEK version). The data key is unchanged.
    pub fn with_rewrapped(&self, id: &str, wrapped: String) -> Result<Self, KmsError> {
        let mut next = self.clone();
        let k = std::iter::once(&mut next.wrapped.current)
            .chain(next.wrapped.previous.iter_mut())
            .find(|k| k.id == id)
            .ok_or_else(|| KmsError::Ring(format!("no data key {id} in the ring")))?;
        k.wrapped = wrapped;
        next.wrapped.validate()?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wk(id: &str) -> WrappedKey {
        WrappedKey {
            id: id.into(),
            provider: "local".into(),
            kek: "kek".into(),
            wrapped: format!("local:fp:{id}"),
            created: 1,
        }
    }

    fn ring_of(id: &str, byte: u8) -> KeyRing {
        KeyRing::from_parts(
            WrappedRing::new(wk(id)),
            [(id.to_string(), DataKey::from_bytes([byte; 32]))],
        )
        .unwrap()
    }

    fn aad(record: &str) -> Aad {
        Aad::new("acme", "identity.totp", record).unwrap()
    }

    #[test]
    fn seals_and_opens() {
        let r = ring_of("k1", 1);
        let s = r.seal(&aad("user-1"), b"JBSWY3DPEHPK3PXP").unwrap();
        assert!(s.starts_with("sv1:k1:"), "{s}");
        assert_ne!(
            s,
            r.seal(&aad("user-1"), b"JBSWY3DPEHPK3PXP").unwrap(),
            "random nonce"
        );
        assert_eq!(
            &r.open(&aad("user-1"), &s).unwrap()[..],
            b"JBSWY3DPEHPK3PXP"
        );
        assert!(!r.needs_reseal(&s));
        // Empty plaintext is fine.
        let e = r.seal(&aad("user-1"), b"").unwrap();
        assert!(r.open(&aad("user-1"), &e).unwrap().is_empty());
    }

    #[test]
    fn swapped_aad_does_not_open() {
        let r = ring_of("k1", 1);
        let s = r.seal(&aad("user-1"), b"secret").unwrap();
        // Another record, another purpose, another organisation: all refused.
        assert_eq!(r.open(&aad("user-2"), &s).unwrap_err(), KmsError::Decrypt);
        let other_purpose = Aad::new("acme", "ingest.agent_ca_key", "user-1").unwrap();
        assert_eq!(r.open(&other_purpose, &s).unwrap_err(), KmsError::Decrypt);
        let other_org = Aad::new("globex", "identity.totp", "user-1").unwrap();
        assert_eq!(r.open(&other_org, &s).unwrap_err(), KmsError::Decrypt);
    }

    #[test]
    fn tampered_or_malformed_does_not_open() {
        let r = ring_of("k1", 1);
        let s = r.seal(&aad("u"), b"secret").unwrap();
        let body = s.strip_prefix("sv1:k1:").unwrap();
        let mut raw =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 1;
        let tampered = format!(
            "sv1:k1:{}",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, raw)
        );
        assert_eq!(r.open(&aad("u"), &tampered).unwrap_err(), KmsError::Decrypt);
        for bad in [
            "sv1:k1:",
            "sv1:k1:!!!",
            "sv1:k1:AAAA",
            "sv1:",
            "plain",
            "sv1:k 1:AAAA",
        ] {
            assert!(
                matches!(r.open(&aad("u"), bad), Err(KmsError::BadSealed(_))),
                "{bad}"
            );
        }
        // Same id, different key bytes: refused.
        assert_eq!(
            ring_of("k1", 9).open(&aad("u"), &s).unwrap_err(),
            KmsError::Decrypt
        );
        // Unknown id: named in the error.
        assert_eq!(
            ring_of("k2", 1).open(&aad("u"), &s).unwrap_err(),
            KmsError::UnknownKey("k1".into())
        );
    }

    #[test]
    fn a_previous_key_opens_but_does_not_seal() {
        let r1 = ring_of("k1", 1);
        let old = r1.seal(&aad("u"), b"old").unwrap();
        let r2 = r1
            .rotated(wk("k2"), DataKey::from_bytes([2; 32]), 100)
            .unwrap();
        assert_eq!(r2.current_id(), "k2");
        assert_eq!(r2.previous_ids(), ["k1"]);
        assert_eq!(r2.rotated_at(), Some(100));
        assert_eq!(&r2.open(&aad("u"), &old).unwrap()[..], b"old");
        assert!(r2.needs_reseal(&old));
        let new = r2.seal(&aad("u"), b"new").unwrap();
        assert!(new.starts_with("sv1:k2:"));
        assert!(!r2.needs_reseal(&new));
        // A rotation to a key the ring already holds is refused.
        assert!(r2
            .rotated(wk("k1"), DataKey::from_bytes([3; 32]), 101)
            .is_err());
        // Retired: the earlier key no longer opens.
        let r3 = r2.retired();
        assert!(r3.previous_ids().is_empty() && r3.rotated_at().is_none());
        assert_eq!(
            r3.open(&aad("u"), &old).unwrap_err(),
            KmsError::UnknownKey("k1".into())
        );
        assert_eq!(&r3.open(&aad("u"), &new).unwrap()[..], b"new");
    }

    #[test]
    fn dxstorage_kms1_is_readable_only_explicitly() {
        let key = DataKey::from_bytes([7; 32]);
        let blob = format::seal_kms1_for_test("vault-abc123", &key, b"s3cr3t");
        let r = KeyRing::from_parts(
            WrappedRing::new(wk("vault-abc123")),
            [("vault-abc123".to_string(), key)],
        )
        .unwrap();
        assert_eq!(&r.open_kms1(&blob).unwrap()[..], b"s3cr3t");
        assert!(r.needs_reseal(&blob));
        // Not through `open`: it carries no associated data.
        assert!(matches!(
            r.open(&aad("u"), &blob),
            Err(KmsError::BadSealed(_))
        ));
        // And `open_kms1` does not take an sv1 blob.
        let sv1 = r.seal(&aad("u"), b"x").unwrap();
        assert!(matches!(r.open_kms1(&sv1), Err(KmsError::BadSealed(_))));
        // A wrong key for that id is refused.
        let wrong = KeyRing::from_parts(
            WrappedRing::new(wk("vault-abc123")),
            [("vault-abc123".to_string(), DataKey::from_bytes([8; 32]))],
        )
        .unwrap();
        assert_eq!(wrong.open_kms1(&blob).unwrap_err(), KmsError::Decrypt);
        // A static key of dxstorage's, outside the ring, opens only.
        let stat = format::seal_kms1_for_test("static-1", &DataKey::from_bytes([5; 32]), b"st");
        let with_static = ring_of("k1", 1)
            .with_open_only("static-1", DataKey::from_bytes([5; 32]))
            .unwrap();
        assert_eq!(&with_static.open_kms1(&stat).unwrap()[..], b"st");
        assert!(with_static
            .seal(&aad("u"), b"x")
            .unwrap()
            .starts_with("sv1:k1:"));
        assert!(ring_of("k1", 1)
            .with_open_only("k1", DataKey::from_bytes([5; 32]))
            .is_err());
    }

    #[test]
    fn a_corrupted_ring_fails_closed() {
        let good = WrappedRing::new(wk("k1")).to_json();
        assert_eq!(WrappedRing::from_json(&good).unwrap().current.id, "k1");
        let not_json = "{\"format\":1,\"current\":";
        assert!(matches!(
            WrappedRing::from_json(not_json),
            Err(KmsError::Ring(_))
        ));
        let future = good.replace("\"format\":1", "\"format\":2");
        assert!(matches!(
            WrappedRing::from_json(&future),
            Err(KmsError::Ring(_))
        ));
        let bad_id = good.replace("\"id\":\"k1\"", "\"id\":\"k:1\"");
        assert!(matches!(
            WrappedRing::from_json(&bad_id),
            Err(KmsError::Ring(_))
        ));
        let empty = good.replace("\"wrapped\":\"local:fp:k1\"", "\"wrapped\":\"\"");
        assert!(matches!(
            WrappedRing::from_json(&empty),
            Err(KmsError::Ring(_))
        ));
        let mut dup = WrappedRing::new(wk("k1"));
        dup.previous.push(wk("k1"));
        assert!(matches!(
            WrappedRing::from_json(&dup.to_json()),
            Err(KmsError::Ring(_))
        ));
        // A ring whose keys are not all unwrapped is refused, not used partially.
        let mut two = WrappedRing::new(wk("k2"));
        two.previous.push(wk("k1"));
        let partial =
            KeyRing::from_parts(two.clone(), [("k2".into(), DataKey::from_bytes([2; 32]))]);
        assert!(matches!(partial, Err(KmsError::Ring(_))));
        // ... and so is a key the ring does not name.
        let extra = KeyRing::from_parts(
            WrappedRing::new(wk("k1")),
            [
                ("k1".into(), DataKey::from_bytes([1; 32])),
                ("k9".into(), DataKey::from_bytes([9; 32])),
            ],
        );
        assert!(matches!(extra, Err(KmsError::Ring(_))));
    }

    #[test]
    fn reads_dxstorage_ring() {
        let f8 = r#"{"id":"vault-abc","vault_key":"dxs","ciphertext":"vault:v2:x","created":7}"#;
        let r = WrappedRing::from_dxstorage_json(f8).unwrap();
        assert_eq!(r.current.id, "vault-abc");
        assert_eq!(r.current.provider, "vault");
        assert_eq!(r.current.kek, "dxs");
        assert_eq!(r.current.wrapped, "vault:v2:x");
        assert!(r.previous.is_empty() && r.rotated_at.is_none());
        let f11 = r#"{"id":"aws-1","vault_key":"arn:k","kms":"aws","ciphertext":"aws:c","created":9,
            "previous":[{"id":"aws-0","vault_key":"arn:k","kms":"aws","ciphertext":"aws:b","created":1}],
            "rotated":8}"#;
        let r = WrappedRing::from_dxstorage_json(f11).unwrap();
        assert_eq!(r.current.provider, "aws");
        assert_eq!(r.previous[0].id, "aws-0");
        assert_eq!(r.rotated_at, Some(8));
        // Round-trips through this crate's own format.
        assert_eq!(WrappedRing::from_json(&r.to_json()).unwrap(), r);
        assert!(WrappedRing::from_dxstorage_json("{}").is_err());
    }

    #[test]
    fn rewrap_keeps_the_data_key() {
        let r = ring_of("k1", 1);
        let s = r.seal(&aad("u"), b"x").unwrap();
        let r2 = r.with_rewrapped("k1", "local:new:zz".into()).unwrap();
        assert_eq!(r2.wrapped().current.wrapped, "local:new:zz");
        assert_eq!(&r2.open(&aad("u"), &s).unwrap()[..], b"x");
        assert!(r.with_rewrapped("nope", "x".into()).is_err());
        assert!(r.with_rewrapped("k1", String::new()).is_err());
        assert_eq!(format!("{r:?}").matches("DataKey").count(), 0);
    }

    #[test]
    fn key_ids_follow_dxstorage() {
        let k = WrappedKey::new("vault", "sentinel", "vault:v1:abc".into(), 5);
        assert!(k.id.starts_with("vault-") && k.id.len() == "vault-".len() + 12);
        assert!(valid_key_id(&k.id));
    }
}
