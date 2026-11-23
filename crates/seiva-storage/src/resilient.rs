//! `ResilientStore` — wraps any [`ObjectStore`] with `seiva-resilience`'s retry,
//! timeout and optional circuit breaker (ADR 0032: "resilience is reused, not
//! reinvented").
//!
//! The decorator distinguishes two kinds of call:
//!
//! - **Idempotent, replayable** (`head`/`get`/`delete`/`list`/`presign`) — wrapped
//!   with retry-with-backoff *and* a per-attempt timeout, so a transient backend
//!   blip is ridden out and a hung call cannot stall the service.
//! - **Streaming / move-consuming** (`put`/`upload_part`/the multipart lifecycle)
//!   — wrapped with a timeout **only**: the body is a one-shot stream, so a blind
//!   retry would replay a half-consumed reader. Idempotency there belongs to the
//!   caller's upload token (ADR 0011), not to a transport retry.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use seiva_ports::Scope;
use seiva_resilience::{BreakerError, CircuitBreaker, RetryPolicy};

use crate::error::{StorageError, StorageResult};
use crate::key::PresignVerb;
use crate::meta::{ObjectMeta, ObjectRef, PresignedUrl};
use crate::store::{ByteReader, MultipartUpload, ObjectStore, PartTag};

/// A resilience-wrapping [`ObjectStore`] decorator: retry + timeout (+ an optional
/// breaker) over an inner store.
#[derive(Clone)]
pub struct ResilientStore<S> {
    inner: S,
    retry: RetryPolicy,
    timeout: Duration,
    breaker: Option<Arc<CircuitBreaker>>,
}

impl<S> ResilientStore<S> {
    /// Wrap `inner` with `retry` and a per-call `timeout`.
    #[must_use]
    pub fn new(inner: S, retry: RetryPolicy, timeout: Duration) -> Self {
        Self {
            inner,
            retry,
            timeout,
            breaker: None,
        }
    }

    /// Also guard every call with a shared circuit breaker.
    #[must_use]
    pub fn with_breaker(mut self, breaker: Arc<CircuitBreaker>) -> Self {
        self.breaker = Some(breaker);
        self
    }

    /// The wrapped store.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// Only a backend/timeout error is worth retrying; a fail-closed scope, an
    /// invalid key, a `404`, an over-size object or a bad signature are permanent
    /// and short-circuit immediately.
    fn is_retryable(error: &StorageError) -> bool {
        matches!(error, StorageError::Backend(_))
    }

    /// Run one attempt under the timeout (and the breaker, if set), mapping a
    /// timeout/open-circuit into a retryable [`StorageError::Backend`].
    async fn attempt<Fut, T>(&self, fut: Fut) -> StorageResult<T>
    where
        Fut: std::future::Future<Output = StorageResult<T>>,
    {
        let timed = async {
            match seiva_resilience::timeout(self.timeout, fut).await {
                Ok(inner) => inner,
                Err(t) => Err(StorageError::Backend(Box::new(t))),
            }
        };
        match &self.breaker {
            None => timed.await,
            Some(breaker) => match breaker.call(timed).await {
                Ok(v) => Ok(v),
                Err(BreakerError::Open) => Err(StorageError::Backend(Box::from("circuit open"))),
                Err(BreakerError::Inner(e)) => Err(e),
            },
        }
    }

    /// Retry a replayable op (retry + per-attempt timeout + optional breaker).
    async fn guarded<F, Fut, T>(&self, make: F) -> StorageResult<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = StorageResult<T>>,
    {
        self.retry
            .retry_if(|| self.attempt(make()), Self::is_retryable)
            .await
    }
}

#[async_trait]
impl<S: ObjectStore> ObjectStore for ResilientStore<S> {
    async fn put(
        &self,
        scope: Option<&Scope>,
        key: &str,
        body: ByteReader,
        meta: ObjectMeta,
    ) -> StorageResult<ObjectMeta> {
        // One-shot body: timeout only, no retry.
        self.attempt(self.inner.put(scope, key, body, meta)).await
    }

    async fn get(
        &self,
        scope: Option<&Scope>,
        key: &str,
    ) -> StorageResult<(ObjectMeta, ByteReader)> {
        self.guarded(|| self.inner.get(scope, key)).await
    }

    async fn head(&self, scope: Option<&Scope>, key: &str) -> StorageResult<ObjectMeta> {
        self.guarded(|| self.inner.head(scope, key)).await
    }

    async fn delete(&self, scope: Option<&Scope>, key: &str) -> StorageResult<()> {
        self.guarded(|| self.inner.delete(scope, key)).await
    }

    async fn list(&self, scope: Option<&Scope>, prefix: &str) -> StorageResult<Vec<ObjectRef>> {
        self.guarded(|| self.inner.list(scope, prefix)).await
    }

    async fn presign(
        &self,
        scope: Option<&Scope>,
        key: &str,
        verb: PresignVerb,
        ttl: Duration,
    ) -> StorageResult<PresignedUrl> {
        self.guarded(|| self.inner.presign(scope, key, verb, ttl))
            .await
    }

    async fn create_multipart(
        &self,
        scope: Option<&Scope>,
        key: &str,
        meta: ObjectMeta,
    ) -> StorageResult<MultipartUpload> {
        // `meta` is moved; retrying would need a clone, so timeout only.
        self.attempt(self.inner.create_multipart(scope, key, meta))
            .await
    }

    async fn upload_part(
        &self,
        upload: &MultipartUpload,
        part_number: u32,
        body: ByteReader,
    ) -> StorageResult<PartTag> {
        self.attempt(self.inner.upload_part(upload, part_number, body))
            .await
    }

    async fn complete_multipart(
        &self,
        upload: MultipartUpload,
        parts: Vec<PartTag>,
    ) -> StorageResult<ObjectMeta> {
        self.attempt(self.inner.complete_multipart(upload, parts))
            .await
    }

    async fn abort_multipart(&self, upload: MultipartUpload) -> StorageResult<()> {
        self.attempt(self.inner.abort_multipart(upload)).await
    }
}
