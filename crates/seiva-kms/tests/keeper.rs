//! The ring's life cycle over the local KEK and the in-memory store: created
//! once, re-wrapped after a KEK rotation, rotated and retired with re-sealing,
//! and refused when corrupted or made by another guardian.
#![cfg(feature = "local")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use seiva_kms::{
    Aad, DataKey, KeyRing, KeyWrapper, KmsError, LocalKek, MemoryRingStore, Reseal, RingKeeper,
    RingStore, WrappedRing,
};

fn kek(lines: &[u8]) -> Arc<LocalKek> {
    let text: String = lines
        .iter()
        .map(|b| {
            format!(
                "{}\n",
                base64::engine::general_purpose::STANDARD.encode([*b; 32])
            )
        })
        .collect();
    Arc::new(LocalKek::parse(&text, "test").unwrap())
}

fn aad(record: &str) -> Aad {
    Aad::new("acme", "identity.totp", record).unwrap()
}

/// A table of sealed secrets, as an application keeps them: (record, blob).
#[derive(Default)]
struct Secrets(Mutex<Vec<(String, String)>>);

impl Secrets {
    fn put(&self, ring: &KeyRing, record: &str, plain: &[u8]) {
        let s = ring.seal(&aad(record), plain).unwrap();
        self.0.lock().unwrap().push((record.into(), s));
    }

    fn blobs(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(_, s)| s.clone())
            .collect()
    }
}

#[async_trait]
impl Reseal for Secrets {
    async fn reseal(&self, _scope: &str, ring: &KeyRing) -> Result<usize, KmsError> {
        let mut n = 0;
        for (record, blob) in self.0.lock().unwrap().iter_mut() {
            if ring.needs_reseal(blob) {
                let plain = ring.open(&aad(record), blob)?;
                *blob = ring.seal(&aad(record), &plain)?;
                n += 1;
            }
        }
        Ok(n)
    }
}

struct Failing;

#[async_trait]
impl Reseal for Failing {
    async fn reseal(&self, _: &str, _: &KeyRing) -> Result<usize, KmsError> {
        Err(KmsError::Store("database down".into()))
    }
}

#[tokio::test]
async fn created_once_then_read() {
    let store = Arc::new(MemoryRingStore::new());
    let keeper = RingKeeper::new(kek(&[1]), store.clone());
    let a = keeper.load("org-1").await.unwrap();
    let b = keeper.load("org-1").await.unwrap();
    assert_eq!(a.current_id(), b.current_id());
    let s = a.seal(&aad("u1"), b"totp").unwrap();
    assert_eq!(&b.open(&aad("u1"), &s).unwrap()[..], b"totp");
    // One ring per organisation: org-2's ring does not open org-1's secret.
    let other = keeper.load("org-2").await.unwrap();
    assert_ne!(other.current_id(), a.current_id());
    assert!(matches!(
        other.open(&aad("u1"), &s),
        Err(KmsError::UnknownKey(_))
    ));
    // The stored form holds no data key.
    let raw = store.raw("org-1").unwrap();
    assert!(raw.contains("\"provider\":\"local\"") && raw.contains("local:"));
}

/// The in-memory store, yielding at every call so two loads interleave.
#[derive(Default)]
struct Yielding(MemoryRingStore);

#[async_trait]
impl RingStore for Yielding {
    async fn load(&self, scope: &str) -> Result<Option<seiva_kms::StoredRing>, KmsError> {
        tokio::task::yield_now().await;
        self.0.load(scope).await
    }

    async fn store(
        &self,
        scope: &str,
        ring: &WrappedRing,
        expected: Option<u64>,
    ) -> Result<bool, KmsError> {
        tokio::task::yield_now().await;
        self.0.store(scope, ring, expected).await
    }
}

#[tokio::test]
async fn concurrent_creation_converges_on_one_ring() {
    let store = Arc::new(Yielding::default());
    let k = kek(&[1]);
    let (ka, kb) = (
        RingKeeper::new(k.clone(), store.clone()),
        RingKeeper::new(k.clone(), store.clone()),
    );
    let (a, b) = tokio::join!(ka.load("org"), kb.load("org"));
    let stored = store.load("org").await.unwrap().unwrap();
    assert_eq!(stored.version, 1, "created once, never overwritten");
    assert_eq!(a.unwrap().current_id(), stored.ring.current.id);
    assert_eq!(b.unwrap().current_id(), stored.ring.current.id);
}

#[tokio::test]
async fn a_kek_rotation_rewraps_the_ring() {
    let store = Arc::new(MemoryRingStore::new());
    let old = kek(&[1]);
    let ring = RingKeeper::new(old.clone(), store.clone())
        .load("org")
        .await
        .unwrap();
    let s = ring.seal(&aad("u"), b"x").unwrap();
    let before = store.load("org").await.unwrap().unwrap();
    assert!(before
        .ring
        .current
        .wrapped
        .starts_with(&format!("local:{}:", old.current_fingerprint())));

    // New KEK on top of the file, old below: the ring is re-wrapped and stored.
    let rotated = kek(&[2, 1]);
    let ring2 = RingKeeper::new(rotated.clone(), store.clone())
        .load("org")
        .await
        .unwrap();
    assert_eq!(
        ring2.current_id(),
        ring.current_id(),
        "the data key is unchanged"
    );
    let after = store.load("org").await.unwrap().unwrap();
    assert!(after
        .ring
        .current
        .wrapped
        .starts_with(&format!("local:{}:", rotated.current_fingerprint())));
    assert!(after.version > before.version);

    // The old KEK line can go now.
    let ring3 = RingKeeper::new(kek(&[2]), store.clone())
        .load("org")
        .await
        .unwrap();
    assert_eq!(&ring3.open(&aad("u"), &s).unwrap()[..], b"x");
    // ... but with only the old KEK, the ring no longer unwraps: fails closed.
    assert!(matches!(
        RingKeeper::new(old, store).load("org").await,
        Err(KmsError::Refused(_))
    ));
}

#[tokio::test]
async fn rotate_reseal_then_retire() {
    let store = Arc::new(MemoryRingStore::new());
    let keeper = RingKeeper::new(kek(&[1]), store.clone());
    let r1 = keeper.load("org").await.unwrap();
    let secrets = Secrets::default();
    secrets.put(&r1, "u1", b"one");
    secrets.put(&r1, "u2", b"two");

    // A failing resealer does not undo the rotation.
    let rot = keeper.rotate("org", &[&Failing]).await.unwrap();
    assert_eq!(rot.reseal_errors.len(), 1);
    assert_eq!(rot.resealed, 0);
    assert_eq!(rot.ring.previous_ids(), [r1.current_id()]);
    // A process that did not hear of the rotation still opens new secrets after
    // a re-read; the old blobs still open with the earlier key.
    let r2 = keeper.load("org").await.unwrap();
    assert_eq!(r2.current_id(), rot.ring.current_id());
    for b in secrets.blobs() {
        assert!(r2.needs_reseal(&b));
    }

    // Not due yet: nothing happens.
    assert!(keeper
        .retire("org", Duration::from_secs(3600), &[&secrets])
        .await
        .unwrap()
        .is_none());
    // A failing resealer aborts the retirement: nothing is dropped.
    assert!(keeper
        .retire("org", Duration::ZERO, &[&secrets, &Failing])
        .await
        .is_err());
    assert_eq!(keeper.load("org").await.unwrap().previous_ids().len(), 1);

    let done = keeper
        .retire("org", Duration::ZERO, &[&secrets])
        .await
        .unwrap()
        .expect("due");
    assert_eq!(done.dropped, [r1.current_id().to_string()]);
    assert!(done.ring.previous_ids().is_empty());
    let r3 = keeper.load("org").await.unwrap();
    assert!(r3.previous_ids().is_empty());
    for (blob, plain) in secrets.blobs().iter().zip([b"one", b"two"]) {
        assert!(!r3.needs_reseal(blob));
        assert_eq!(
            &r3.open(&aad(if plain == b"one" { "u1" } else { "u2" }), blob)
                .unwrap()[..],
            plain
        );
    }
    // Nothing left to retire.
    assert!(keeper
        .retire("org", Duration::ZERO, &[&secrets])
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn scheduled_rotation_only_when_old() {
    let keeper = RingKeeper::new(kek(&[1]), Arc::new(MemoryRingStore::new()));
    let r = keeper.load("org").await.unwrap();
    assert!(keeper
        .rotate_if_older("org", Duration::from_secs(3600), &[])
        .await
        .unwrap()
        .is_none());
    let rot = keeper
        .rotate_if_older("org", Duration::ZERO, &[])
        .await
        .unwrap()
        .expect("rotated");
    assert_ne!(rot.ring.current_id(), r.current_id());
}

/// A store that changes the ring between read and write (another replica).
struct Racing {
    inner: MemoryRingStore,
    other: Arc<LocalKek>,
}

#[async_trait]
impl RingStore for Racing {
    async fn load(&self, scope: &str) -> Result<Option<seiva_kms::StoredRing>, KmsError> {
        self.inner.load(scope).await
    }

    async fn store(
        &self,
        scope: &str,
        ring: &WrappedRing,
        expected: Option<u64>,
    ) -> Result<bool, KmsError> {
        if expected.is_some() {
            // Someone else rotates first.
            let stored = self.inner.load(scope).await?.unwrap();
            let mut theirs = stored.ring.clone();
            let g = self.other.generate().await?;
            theirs.previous.insert(0, theirs.current.clone());
            theirs.current = seiva_kms::WrappedKey::new("local", "local", g.wrapped, 1);
            assert!(
                self.inner
                    .store(scope, &theirs, Some(stored.version))
                    .await?
            );
        }
        self.inner.store(scope, ring, expected).await
    }
}

#[tokio::test]
async fn a_concurrent_rotation_is_a_conflict() {
    let k = kek(&[1]);
    let store = Arc::new(Racing {
        inner: MemoryRingStore::new(),
        other: k.clone(),
    });
    let keeper = RingKeeper::new(k, store);
    keeper.load("org").await.unwrap();
    assert_eq!(
        keeper.rotate("org", &[]).await.unwrap_err(),
        KmsError::Conflict
    );
}

#[tokio::test]
async fn a_corrupted_or_foreign_ring_fails_closed() {
    let store = Arc::new(MemoryRingStore::new());
    let keeper = RingKeeper::new(kek(&[1]), store.clone());
    keeper.load("org").await.unwrap();
    let good = store.raw("org").unwrap();

    // Garbage in the store: an error, never a fresh ring over it.
    store.put_raw("org", "{\"format\":1,");
    assert!(matches!(keeper.load("org").await, Err(KmsError::Ring(_))));
    assert_eq!(store.raw("org").unwrap(), "{\"format\":1,");

    // A wrapped key that was tampered with does not unwrap.
    let tampered = good.replacen("\"wrapped\":\"local:", "\"wrapped\":\"local:00", 1);
    store.put_raw("org", &tampered);
    assert!(keeper.load("org").await.is_err());

    // A ring made by another KEK name or guardian is refused.
    store.put_raw("org", &good);
    let renamed = RingKeeper::new(
        Arc::new(
            LocalKek::from_keys(vec![DataKey::from_bytes([1; 32])])
                .unwrap()
                .named("other"),
        ),
        store.clone(),
    );
    assert!(matches!(
        renamed.load("org").await,
        Err(KmsError::Config(_))
    ));
    store.put_raw(
        "org",
        &good.replace("\"provider\":\"local\"", "\"provider\":\"vault\""),
    );
    assert!(matches!(keeper.load("org").await, Err(KmsError::Config(_))));

    // Rotating a scope without a ring is an error, not a silent creation.
    assert!(matches!(
        keeper.rotate("nobody", &[]).await,
        Err(KmsError::Ring(_))
    ));
}

/// A guardian that is down for the first `n` calls.
struct Flaky {
    inner: Arc<LocalKek>,
    down: Mutex<u32>,
}

#[async_trait]
impl KeyWrapper for Flaky {
    fn provider(&self) -> &'static str {
        "local"
    }
    fn kek_id(&self) -> &str {
        self.inner.kek_id()
    }
    async fn generate(&self) -> Result<seiva_kms::GeneratedKey, KmsError> {
        {
            let mut d = self.down.lock().unwrap();
            if *d > 0 {
                *d -= 1;
                return Err(KmsError::Unavailable("connection refused".into()));
            }
        }
        self.inner.generate().await
    }
    async fn unwrap(&self, k: &str, w: &str) -> Result<DataKey, KmsError> {
        self.inner.unwrap(k, w).await
    }
    async fn rewrap(&self, k: &str, w: &str, p: &DataKey) -> Result<Option<String>, KmsError> {
        self.inner.rewrap(k, w, p).await
    }
    async fn health(&self) -> Result<(), KmsError> {
        Ok(())
    }
}

#[tokio::test]
async fn boot_retries_an_unavailable_guardian_until_its_deadline() {
    let flaky = |n| {
        Arc::new(Flaky {
            inner: kek(&[1]),
            down: Mutex::new(n),
        })
    };
    // Down twice, up within the deadline: boots.
    let keeper = RingKeeper::new(flaky(2), Arc::new(MemoryRingStore::new()));
    assert!(keeper.boot("org", Duration::from_secs(5)).await.is_ok());
    // Down past the deadline: fails closed with the transient error.
    let keeper = RingKeeper::new(flaky(100), Arc::new(MemoryRingStore::new()));
    let e = keeper
        .boot("org", Duration::from_millis(800))
        .await
        .unwrap_err();
    assert!(e.is_transient(), "{e}");
}
