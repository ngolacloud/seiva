//! The ports through which the caller keeps the wrapped ring and re-seals its
//! secrets. This crate never decides where either lives.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::{KeyRing, KmsError, WrappedRing};

/// A stored ring and the version it was read at (for the next write).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRing {
    /// The wrapped ring.
    pub ring: WrappedRing,
    /// Opaque to this crate; the store bumps it on every write.
    pub version: u64,
}

/// Where the caller keeps wrapped rings: one per `scope` (an organisation, a
/// deployment). dxstorage keeps it in its cluster store, the Sentinel in a
/// Postgres row with a `version` column.
///
/// Writes are **compare-and-swap**, so two processes that rotate at once never
/// both win: the loser gets `false` and re-reads.
#[async_trait::async_trait]
pub trait RingStore: Send + Sync {
    /// The stored ring of `scope`, if any. A ring that does not parse is an
    /// error ([`KmsError::Ring`]), never `None`: a corrupted ring must not be
    /// silently replaced by a new one (that would orphan every secret).
    async fn load(&self, scope: &str) -> Result<Option<StoredRing>, KmsError>;

    /// Stores `ring` for `scope` if the stored version is still `expected`
    /// (`None`: only if there is no ring yet). `false` when someone else wrote
    /// first.
    async fn store(
        &self,
        scope: &str,
        ring: &WrappedRing,
        expected: Option<u64>,
    ) -> Result<bool, KmsError>;
}

/// Whatever stores sealed secrets: re-seals them with the ring's current key
/// after a rotation, and before the earlier keys are retired.
#[async_trait::async_trait]
pub trait Reseal: Send + Sync {
    /// Re-seals every secret of `scope` that `ring`'s current key does not seal
    /// ([`KeyRing::needs_reseal`]), opening it with `ring` and the secret's own
    /// associated data; returns how many changed.
    async fn reseal(&self, scope: &str, ring: &KeyRing) -> Result<usize, KmsError>;
}

/// An in-memory [`RingStore`], for tests and single-process tools.
#[derive(Debug, Default)]
pub struct MemoryRingStore {
    rings: Mutex<HashMap<String, (String, u64)>>,
}

impl MemoryRingStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// The stored JSON of `scope` (to inspect or corrupt it in a test).
    pub fn raw(&self, scope: &str) -> Option<String> {
        self.rings
            .lock()
            .expect("lock")
            .get(scope)
            .map(|(j, _)| j.clone())
    }

    /// Replaces the stored JSON of `scope` as-is (bumping the version).
    pub fn put_raw(&self, scope: &str, json: &str) {
        let mut g = self.rings.lock().expect("lock");
        let v = g.get(scope).map(|(_, v)| v + 1).unwrap_or(1);
        g.insert(scope.to_string(), (json.to_string(), v));
    }
}

#[async_trait::async_trait]
impl RingStore for MemoryRingStore {
    async fn load(&self, scope: &str) -> Result<Option<StoredRing>, KmsError> {
        let g = self.rings.lock().expect("lock");
        match g.get(scope) {
            None => Ok(None),
            Some((json, version)) => Ok(Some(StoredRing {
                ring: WrappedRing::from_json(json)?,
                version: *version,
            })),
        }
    }

    async fn store(
        &self,
        scope: &str,
        ring: &WrappedRing,
        expected: Option<u64>,
    ) -> Result<bool, KmsError> {
        let mut g = self.rings.lock().expect("lock");
        let current = g.get(scope).map(|(_, v)| *v);
        if current != expected {
            return Ok(false);
        }
        g.insert(
            scope.to_string(),
            (ring.to_json(), current.map(|v| v + 1).unwrap_or(1)),
        );
        Ok(true)
    }
}
