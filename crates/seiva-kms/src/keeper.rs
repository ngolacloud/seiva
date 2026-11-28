//! `RingKeeper`: the life cycle of a ring over the three ports — create it on
//! first use, unwrap it at start-up (retrying an unreachable guardian until a
//! deadline), re-wrap it after the guardian rotated the KEK, rotate the data
//! key, and retire the earlier keys once everything is re-sealed.
//!
//! It holds no state between calls: caching the [`KeyRing`], re-reading it on a
//! notification or a timer, and locking across replicas belong to the caller
//! (the store's compare-and-swap keeps concurrent writers safe regardless).

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::{
    DataKey, KeyRing, KeyWrapper, KmsError, Reseal, RingStore, StoredRing, WrappedKey, WrappedRing,
};

/// What a rotation did.
#[derive(Debug)]
pub struct Rotation {
    /// The ring now stored: the new key seals, the earlier ones only open.
    pub ring: KeyRing,
    /// Secrets the resealers re-sealed with the new key.
    pub resealed: usize,
    /// Resealers that failed. The rotation itself stands (the earlier keys
    /// still open); [`RingKeeper::retire`] re-seals again before dropping them.
    pub reseal_errors: Vec<KmsError>,
}

/// What a retirement did.
#[derive(Debug)]
pub struct Retirement {
    /// The ring now stored: the current key only.
    pub ring: KeyRing,
    /// The ids of the keys dropped.
    pub dropped: Vec<String>,
    /// Secrets re-sealed first.
    pub resealed: usize,
}

/// Loads, rotates and retires the ring of a scope through a [`KeyWrapper`] and
/// a [`RingStore`].
#[derive(Clone)]
pub struct RingKeeper {
    wrapper: Arc<dyn KeyWrapper>,
    store: Arc<dyn RingStore>,
}

impl std::fmt::Debug for RingKeeper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RingKeeper")
            .field("provider", &self.wrapper.provider())
            .field("kek", &self.wrapper.kek_id())
            .finish_non_exhaustive()
    }
}

/// Unix seconds now.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl RingKeeper {
    /// A keeper over `wrapper` (the guardian) and `store` (where rings live).
    pub fn new(wrapper: Arc<dyn KeyWrapper>, store: Arc<dyn RingStore>) -> Self {
        Self { wrapper, store }
    }

    /// The guardian.
    pub fn wrapper(&self) -> &dyn KeyWrapper {
        self.wrapper.as_ref()
    }

    /// The ring of `scope`, unwrapped: created on first use (one process wins
    /// the race, the others use its ring), otherwise read and unwrapped. A key
    /// the guardian now wraps with a newer KEK version is re-wrapped and stored
    /// (best effort: not needed to run).
    pub async fn load(&self, scope: &str) -> Result<KeyRing, KmsError> {
        for _ in 0..3 {
            match self.store.load(scope).await? {
                Some(stored) => return self.open_stored(scope, stored, true).await,
                None => {
                    let ring = self.new_ring().await?;
                    if self.store.store(scope, ring.wrapped(), None).await? {
                        return Ok(ring);
                    }
                    // Another process created it first: use that one.
                }
            }
        }
        Err(KmsError::Conflict)
    }

    /// [`Self::load`], retrying an unavailable guardian (and only that) with
    /// back-off until `timeout`. Fails closed: past the deadline the last error
    /// is returned and the caller must not report ready.
    pub async fn boot(&self, scope: &str, timeout: Duration) -> Result<KeyRing, KmsError> {
        retry_until(timeout, || self.load(scope)).await
    }

    /// Rotates the data key of `scope`: a new key from the guardian seals from
    /// now on, the current one joins the earlier keys, the ring is stored
    /// (compare-and-swap: [`KmsError::Conflict`] if another process changed it
    /// meanwhile), then every resealer re-seals with the new key.
    pub async fn rotate(
        &self,
        scope: &str,
        resealers: &[&dyn Reseal],
    ) -> Result<Rotation, KmsError> {
        let stored = self.stored(scope).await?;
        let version = stored.version;
        let ring = self.open_stored(scope, stored, false).await?;
        let fresh = self.wrapper.generate().await?;
        let next = ring.rotated(
            WrappedKey::new(
                self.wrapper.provider(),
                self.wrapper.kek_id(),
                fresh.wrapped,
                now(),
            ),
            fresh.plain,
            now(),
        )?;
        if !self
            .store
            .store(scope, next.wrapped(), Some(version))
            .await?
        {
            return Err(KmsError::Conflict);
        }
        let mut resealed = 0;
        let mut reseal_errors = Vec::new();
        for r in resealers {
            match r.reseal(scope, &next).await {
                Ok(n) => resealed += n,
                Err(e) => reseal_errors.push(e),
            }
        }
        Ok(Rotation {
            ring: next,
            resealed,
            reseal_errors,
        })
    }

    /// The scheduled rotation: [`Self::rotate`] only when the current key is
    /// older than `max_age` (`None` otherwise).
    pub async fn rotate_if_older(
        &self,
        scope: &str,
        max_age: Duration,
        resealers: &[&dyn Reseal],
    ) -> Result<Option<Rotation>, KmsError> {
        let stored = self.stored(scope).await?;
        if now() - stored.ring.current.created < max_age.as_secs() as i64 {
            return Ok(None);
        }
        self.rotate(scope, resealers).await.map(Some)
    }

    /// Drops the earlier keys of `scope` once `after` has passed since the
    /// rotation: every resealer re-seals first (any failure aborts, nothing is
    /// dropped), then the ring is stored with the current key only.
    ///
    /// `after` must exceed the time every process takes to pick up a rotation
    /// (its re-read interval): a process still holding the old ring would
    /// otherwise seal with a key about to be dropped.
    pub async fn retire(
        &self,
        scope: &str,
        after: Duration,
        resealers: &[&dyn Reseal],
    ) -> Result<Option<Retirement>, KmsError> {
        let stored = self.stored(scope).await?;
        let due = !stored.ring.previous.is_empty()
            && stored.ring.rotated_at.unwrap_or(0) + after.as_secs() as i64 <= now();
        if !due {
            return Ok(None);
        }
        let version = stored.version;
        let ring = self.open_stored(scope, stored, false).await?;
        let mut resealed = 0;
        for r in resealers {
            resealed += r.reseal(scope, &ring).await?;
        }
        let dropped = ring.previous_ids().iter().map(|s| s.to_string()).collect();
        let next = ring.retired();
        if !self
            .store
            .store(scope, next.wrapped(), Some(version))
            .await?
        {
            return Err(KmsError::Conflict);
        }
        Ok(Some(Retirement {
            ring: next,
            dropped,
            resealed,
        }))
    }

    async fn stored(&self, scope: &str) -> Result<StoredRing, KmsError> {
        self.store
            .load(scope)
            .await?
            .ok_or_else(|| KmsError::Ring(format!("no key ring for {scope} yet")))
    }

    async fn new_ring(&self) -> Result<KeyRing, KmsError> {
        let fresh = self.wrapper.generate().await?;
        let key = WrappedKey::new(
            self.wrapper.provider(),
            self.wrapper.kek_id(),
            fresh.wrapped,
            now(),
        );
        let id = key.id.clone();
        KeyRing::from_parts(WrappedRing::new(key), [(id, fresh.plain)])
    }

    /// The ring was made with this guardian and KEK (never another).
    fn check(&self, ring: &WrappedRing) -> Result<(), KmsError> {
        if let Some(k) = ring.keys().find(|k| k.provider != self.wrapper.provider()) {
            return Err(KmsError::Config(format!(
                "data key {} is wrapped by the {} guardian, but this process is configured for {}",
                k.id,
                k.provider,
                self.wrapper.provider()
            )));
        }
        if ring.current.kek != self.wrapper.kek_id() {
            return Err(KmsError::Config(format!(
                "the key ring is wrapped by {} KEK '{}', but the configured KEK is '{}'",
                self.wrapper.provider(),
                ring.current.kek,
                self.wrapper.kek_id()
            )));
        }
        Ok(())
    }

    /// Unwraps every key of `stored`; with `rewrap`, also re-wraps keys under an
    /// older KEK version and stores the result (compare-and-swap; losing the
    /// race is fine, the other writer's ring is read next time).
    async fn open_stored(
        &self,
        scope: &str,
        stored: StoredRing,
        rewrap: bool,
    ) -> Result<KeyRing, KmsError> {
        self.check(&stored.ring)?;
        let mut keys: Vec<(String, DataKey)> = Vec::new();
        for k in stored.ring.keys() {
            keys.push((k.id.clone(), self.wrapper.unwrap(&k.kek, &k.wrapped).await?));
        }
        let mut ring = KeyRing::from_parts(stored.ring.clone(), keys)?;
        if !rewrap {
            return Ok(ring);
        }
        let mut changed = false;
        for k in stored.ring.keys() {
            let plain = ring_key(&ring, &k.id);
            // A failed re-wrap is not needed to run; the next start tries again.
            if let Ok(Some(w)) = self.wrapper.rewrap(&k.kek, &k.wrapped, &plain).await {
                ring = ring.with_rewrapped(&k.id, w)?;
                changed = true;
            }
        }
        if changed {
            // A failed write (or a lost race) is fine: the ring is usable as it is.
            let _ = self
                .store
                .store(scope, ring.wrapped(), Some(stored.version))
                .await;
        }
        Ok(ring)
    }
}

/// The unwrapped key `id` of `ring` (present: `from_parts` checked it).
fn ring_key(ring: &KeyRing, id: &str) -> DataKey {
    ring.key(id)
        .expect("every key of the ring is unwrapped")
        .clone()
}

/// Runs `f` until it succeeds, fails with a non-transient error, or `timeout`
/// passes; between attempts it waits 0.5 s, doubling up to 8 s.
pub async fn retry_until<T, F, Fut>(timeout: Duration, mut f: F) -> Result<T, KmsError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, KmsError>>,
{
    let deadline = Instant::now() + timeout;
    let mut delay = Duration::from_millis(500);
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if e.is_transient() && Instant::now() + delay < deadline => {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(8));
            }
            Err(e) => return Err(e),
        }
    }
}
