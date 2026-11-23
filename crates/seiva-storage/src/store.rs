//! The `ObjectStore` port: byte-oriented, streaming both ways, vendor-neutral.
//!
//! Every method takes the ambient [`Scope`] explicitly so tenant isolation is a
//! parameter the caller cannot forget and the store cannot skip — a `None`
//! scope fails closed. The payload crosses as a boxed [`ByteReader`] so the
//! trait stays object-safe (`Arc<dyn ObjectStore>`) and no method ever
//! materialises a whole object in memory.

use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use seiva_ports::Scope;
use tokio::io::AsyncRead;

use crate::error::StorageResult;
use crate::key::PresignVerb;
use crate::meta::{ObjectMeta, ObjectRef, PresignedUrl};

/// A streaming byte source/sink: boxed so the port is object-safe, `Unpin` so an
/// adapter can `tokio::io::copy` it without pinning gymnastics.
pub type ByteReader = Pin<Box<dyn AsyncRead + Send + Unpin>>;

/// Box an `AsyncRead` into the [`ByteReader`] the port streams over.
pub fn byte_reader<R>(reader: R) -> ByteReader
where
    R: AsyncRead + Send + Unpin + 'static,
{
    Box::pin(reader)
}

/// An in-progress multipart upload: an opaque handle the adapter mints at
/// [`create_multipart`](ObjectStore::create_multipart) and the caller carries
/// through [`upload_part`](ObjectStore::upload_part) and
/// [`complete_multipart`](ObjectStore::complete_multipart).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultipartUpload {
    /// The adapter-chosen upload id (a staging directory for local-fs, the S3
    /// `UploadId` for S3).
    pub upload_id: String,
    /// The resolved, tenant-prefixed destination key — captured at creation so
    /// later parts need no re-resolution (and cannot be redirected).
    pub resolved_key: String,
    /// The metadata the completed object will carry.
    pub meta: ObjectMeta,
}

/// A receipt for one uploaded part: the part number and its entity tag, replayed
/// in order at [`complete_multipart`](ObjectStore::complete_multipart).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartTag {
    /// The 1-based part number.
    pub part_number: u32,
    /// The part's entity tag (a content digest for local-fs, the S3 `ETag`).
    pub etag: String,
}

/// The pluggable object-store capability: streaming blobs, tenant-keyed and
/// fail-closed. Bound at the composition root to a local-fs or S3 adapter.
#[async_trait]
pub trait ObjectStore: Send + Sync {
    /// Stream `body` to `key` under the ambient `scope`, validating `meta`
    /// against the store's limits first. Returns a meta carrying the **actual**
    /// bytes written.
    ///
    /// # Errors
    ///
    /// Fails closed on a `None` scope; rejects an unsafe key, a disallowed
    /// content-type, or an object past the size limit.
    async fn put(
        &self,
        scope: Option<&Scope>,
        key: &str,
        body: ByteReader,
        meta: ObjectMeta,
    ) -> StorageResult<ObjectMeta>;

    /// Open `key` for streaming download: the metadata plus a [`ByteReader`] the
    /// caller drains. The whole object is never buffered.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`](crate::StorageError::NotFound) if absent; fails
    /// closed on a `None` scope.
    async fn get(
        &self,
        scope: Option<&Scope>,
        key: &str,
    ) -> StorageResult<(ObjectMeta, ByteReader)>;

    /// The metadata for `key` without transferring its bytes.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`](crate::StorageError::NotFound) if absent; fails
    /// closed on a `None` scope.
    async fn head(&self, scope: Option<&Scope>, key: &str) -> StorageResult<ObjectMeta>;

    /// Delete `key`. Deleting an absent key is **idempotent** (returns `Ok`).
    ///
    /// # Errors
    ///
    /// Fails closed on a `None` scope; surfaces a backend error.
    async fn delete(&self, scope: Option<&Scope>, key: &str) -> StorageResult<()>;

    /// List objects under `prefix`, returning caller-visible keys (the tenant
    /// prefix stripped) and their sizes.
    ///
    /// # Errors
    ///
    /// Fails closed on a `None` scope; surfaces a backend error.
    async fn list(&self, scope: Option<&Scope>, prefix: &str) -> StorageResult<Vec<ObjectRef>>;

    /// Mint a short-TTL presigned URL that authorises `verb` on `key`, delegating
    /// the transfer to the store so bytes flow client↔storage directly.
    ///
    /// # Errors
    ///
    /// Fails closed on a `None` scope; rejects an unsafe key.
    async fn presign(
        &self,
        scope: Option<&Scope>,
        key: &str,
        verb: PresignVerb,
        ttl: Duration,
    ) -> StorageResult<PresignedUrl>;

    /// Begin a multipart upload for a large object, validating `meta` up front.
    ///
    /// # Errors
    ///
    /// Fails closed on a `None` scope; rejects an unsafe key or disallowed meta.
    async fn create_multipart(
        &self,
        scope: Option<&Scope>,
        key: &str,
        meta: ObjectMeta,
    ) -> StorageResult<MultipartUpload>;

    /// Stream one part of `upload`; parts are bounded in memory, not the whole
    /// object. Returns the [`PartTag`] to replay at completion.
    ///
    /// # Errors
    ///
    /// [`StorageError::NoSuchUpload`](crate::StorageError::NoSuchUpload) for an
    /// unknown upload; surfaces a backend error.
    async fn upload_part(
        &self,
        upload: &MultipartUpload,
        part_number: u32,
        body: ByteReader,
    ) -> StorageResult<PartTag>;

    /// Assemble the parts (in `parts` order) into the final object and return its
    /// meta. The streamed total is re-checked against the size limit.
    ///
    /// # Errors
    ///
    /// [`StorageError::NoSuchUpload`](crate::StorageError::NoSuchUpload) for an
    /// unknown upload; [`StorageError::TooLarge`](crate::StorageError::TooLarge)
    /// if the assembled object exceeds the limit.
    async fn complete_multipart(
        &self,
        upload: MultipartUpload,
        parts: Vec<PartTag>,
    ) -> StorageResult<ObjectMeta>;

    /// Abandon a multipart upload, discarding any staged parts. Idempotent.
    ///
    /// # Errors
    ///
    /// Surfaces a backend error.
    async fn abort_multipart(&self, upload: MultipartUpload) -> StorageResult<()>;
}
