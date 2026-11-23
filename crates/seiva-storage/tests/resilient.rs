//! Proofs for the `ResilientStore` decorator: a transient backend error is
//! retried to success, a permanent error short-circuits, and a hung call times
//! out — reusing `seiva-resilience`, not reinventing it.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use seiva_ports::Scope;
use seiva_resilience::RetryPolicy;
use seiva_storage::{
    ByteReader, MultipartUpload, ObjectMeta, ObjectRef, ObjectStore, PartTag, PresignVerb,
    PresignedUrl, ResilientStore, StorageError, StorageResult,
};

/// A store whose `head` fails `fail_times` with a (retryable) backend error, or
/// — if `permanent` — always fails with a fail-closed `MissingScope` instead.
/// Every other method is unused by these tests.
struct ScriptedStore {
    calls: AtomicU32,
    fail_times: u32,
    permanent: bool,
    delay: Duration,
}

impl ScriptedStore {
    fn new(fail_times: u32) -> Self {
        Self {
            calls: AtomicU32::new(0),
            fail_times,
            permanent: false,
            delay: Duration::ZERO,
        }
    }
    fn permanent() -> Self {
        Self {
            calls: AtomicU32::new(0),
            fail_times: 0,
            permanent: true,
            delay: Duration::ZERO,
        }
    }
    fn slow(delay: Duration) -> Self {
        Self {
            calls: AtomicU32::new(0),
            fail_times: 0,
            permanent: false,
            delay,
        }
    }
    fn count(&self) -> u32 {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl ObjectStore for ScriptedStore {
    async fn put(
        &self,
        _s: Option<&Scope>,
        _k: &str,
        _b: ByteReader,
        m: ObjectMeta,
    ) -> StorageResult<ObjectMeta> {
        Ok(m)
    }

    async fn get(&self, _s: Option<&Scope>, _k: &str) -> StorageResult<(ObjectMeta, ByteReader)> {
        Err(StorageError::NotFound)
    }

    async fn head(&self, _s: Option<&Scope>, _k: &str) -> StorageResult<ObjectMeta> {
        let n = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
        if self.delay > Duration::ZERO {
            tokio::time::sleep(self.delay).await;
        }
        if self.permanent {
            return Err(StorageError::MissingScope);
        }
        if n <= self.fail_times {
            return Err(StorageError::Backend(Box::from("transient blip")));
        }
        Ok(ObjectMeta::new("text/plain", 1))
    }

    async fn delete(&self, _s: Option<&Scope>, _k: &str) -> StorageResult<()> {
        Ok(())
    }
    async fn list(&self, _s: Option<&Scope>, _p: &str) -> StorageResult<Vec<ObjectRef>> {
        Ok(vec![])
    }
    async fn presign(
        &self,
        _s: Option<&Scope>,
        _k: &str,
        _v: PresignVerb,
        _t: Duration,
    ) -> StorageResult<PresignedUrl> {
        Ok(PresignedUrl::new("local://x/y?verb=GET&exp=0&sig=0"))
    }
    async fn create_multipart(
        &self,
        _s: Option<&Scope>,
        _k: &str,
        m: ObjectMeta,
    ) -> StorageResult<MultipartUpload> {
        Ok(MultipartUpload {
            upload_id: "0".into(),
            resolved_key: "k".into(),
            meta: m,
        })
    }
    async fn upload_part(
        &self,
        _u: &MultipartUpload,
        part_number: u32,
        _b: ByteReader,
    ) -> StorageResult<PartTag> {
        Ok(PartTag {
            part_number,
            etag: "0".into(),
        })
    }
    async fn complete_multipart(
        &self,
        u: MultipartUpload,
        _p: Vec<PartTag>,
    ) -> StorageResult<ObjectMeta> {
        Ok(u.meta)
    }
    async fn abort_multipart(&self, _u: MultipartUpload) -> StorageResult<()> {
        Ok(())
    }
}

/// A zero-backoff policy so retries do not actually sleep in the test.
fn fast_retry(max_attempts: u32) -> RetryPolicy {
    RetryPolicy {
        max_attempts,
        backoff_base_ms: 0,
    }
}

#[tokio::test]
async fn retries_a_transient_backend_error_to_success() {
    let inner = ScriptedStore::new(2);
    let store = ResilientStore::new(inner, fast_retry(5), Duration::from_secs(5));

    let meta = store.head(None, "k").await.unwrap();
    assert_eq!(meta.content_length, 1);
    assert_eq!(store.inner().count(), 3, "two failures then one success");
}

#[tokio::test]
async fn gives_up_after_exhausting_attempts() {
    let inner = ScriptedStore::new(10);
    let store = ResilientStore::new(inner, fast_retry(3), Duration::from_secs(5));

    let err = store.head(None, "k").await.unwrap_err();
    assert!(matches!(err, StorageError::Backend(_)));
    assert_eq!(store.inner().count(), 3, "capped at max_attempts");
}

#[tokio::test]
async fn permanent_error_is_not_retried() {
    let inner = ScriptedStore::permanent();
    let store = ResilientStore::new(inner, fast_retry(5), Duration::from_secs(5));

    let err = store.head(None, "k").await.unwrap_err();
    assert!(matches!(err, StorageError::MissingScope));
    assert_eq!(
        store.inner().count(),
        1,
        "a fail-closed error short-circuits"
    );
}

#[tokio::test(start_paused = true)]
async fn a_hung_call_times_out() {
    let inner = ScriptedStore::slow(Duration::from_secs(30));
    // One attempt, a 1s timeout: the 30s call is cut off and reported as backend.
    let store = ResilientStore::new(inner, fast_retry(1), Duration::from_secs(1));

    let err = store.head(None, "k").await.unwrap_err();
    assert!(matches!(err, StorageError::Backend(_)));
}
