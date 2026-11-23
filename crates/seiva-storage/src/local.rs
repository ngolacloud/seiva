//! `LocalFsObjectStore` — the default, dev/offline adapter: a directory tree
//! confined to its tenant prefix.
//!
//! Every key is resolved under `tenant/{id}/…` (ADR 0032) and every filesystem
//! path is **re-confined** after resolution — the parent directory is
//! canonicalised and checked to stay under the store root, so a symlink planted
//! inside the tree cannot redirect a write or read out of it. Objects stream to
//! and from disk; a sidecar file carries the content-type and user metadata.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use seiva_ports::{Clock, Scope};
use sha2::{Digest, Sha256};
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{StorageError, StorageResult};
use crate::key::{resolve_key, PresignVerb, META_SUFFIX};
use crate::meta::{ObjectMeta, ObjectRef, PresignedUrl, StoreLimits};
use crate::store::{ByteReader, MultipartUpload, ObjectStore, PartTag};

/// The default signing key used for presigned URLs when none is configured — a
/// visible dev placeholder; a browser-facing deployment MUST call
/// [`with_signing_key`](LocalFsObjectStore::with_signing_key).
const DEV_SIGNING_KEY: [u8; 32] = *b"seiva-storage-dev-presign-key!!!";

/// The directory, under the store root, where multipart parts are staged.
const UPLOADS_DIR: &str = ".uploads";

/// The system clock: the production [`Clock`] the store reads presign expiries
/// from. Injected so a test pins time and proves expiry deterministically.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// A path-confined, tenant-aware local object store rooted at a directory.
#[derive(Clone)]
pub struct LocalFsObjectStore {
    root: PathBuf,
    limits: StoreLimits,
    signing_key: [u8; 32],
    clock: Arc<dyn Clock>,
    /// The authority segment of a presigned `local://…` URL.
    store_id: String,
    /// A per-process counter so concurrent writes and multipart uploads get
    /// distinct temp/staging names without a random source.
    seq: Arc<AtomicU64>,
}

impl LocalFsObjectStore {
    /// Open (creating if missing) a local store rooted at `root`.
    ///
    /// # Errors
    ///
    /// Surfaces an I/O error creating or canonicalising the root.
    pub async fn new(root: impl AsRef<Path>) -> StorageResult<Self> {
        let root = root.as_ref();
        fs::create_dir_all(root).await?;
        let root = fs::canonicalize(root).await?;
        Ok(Self {
            root,
            limits: StoreLimits::default(),
            signing_key: DEV_SIGNING_KEY,
            clock: Arc::new(SystemClock),
            store_id: "local".to_string(),
            seq: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Set the size/content-type limits a `put` is validated against.
    #[must_use]
    pub fn with_limits(mut self, limits: StoreLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Set the presigned-URL signing key (any length; hashed to 32 bytes).
    #[must_use]
    pub fn with_signing_key(mut self, key: impl AsRef<[u8]>) -> Self {
        self.signing_key = Sha256::digest(key.as_ref()).into();
        self
    }

    /// Inject a [`Clock`] so presign expiries are deterministic under test.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Set the authority segment of the `local://…` presigned URLs this store mints.
    #[must_use]
    pub fn with_store_id(mut self, store_id: impl Into<String>) -> Self {
        self.store_id = store_id.into();
        self
    }

    /// The tenant/global prefix a scope resolves to, independent of any key.
    fn scope_prefix(scope: Option<&Scope>) -> StorageResult<String> {
        match scope {
            Some(Scope::Tenant(t)) => Ok(format!("tenant/{}", t.as_str())),
            Some(Scope::Global) => Ok("shared".to_string()),
            // `None` and any future, unresolvable scope kind: fail closed.
            _ => Err(StorageError::MissingScope),
        }
    }

    /// Join `resolved_key` onto the root and re-confine it: the parent directory
    /// is canonicalised and verified to stay under the root, defeating a symlink
    /// planted inside the tree. With `create_parents`, missing directories are
    /// created first so a write can confine its (not-yet-existing) parent.
    async fn confine(&self, resolved_key: &str, create_parents: bool) -> StorageResult<PathBuf> {
        let full = self.root.join(resolved_key);
        let parent = full.parent().ok_or(StorageError::InvalidKey {
            reason: "key has no parent",
        })?;
        if create_parents {
            fs::create_dir_all(parent).await?;
        }
        let parent_canon = fs::canonicalize(parent).await?;
        if !parent_canon.starts_with(&self.root) {
            return Err(StorageError::InvalidKey {
                reason: "resolved path escapes store root",
            });
        }
        let name = full.file_name().ok_or(StorageError::InvalidKey {
            reason: "key has no file name",
        })?;
        Ok(parent_canon.join(name))
    }

    /// The sidecar path carrying a `put`'s content-type and user metadata.
    fn sidecar_of(full: &Path) -> PathBuf {
        let mut s = full.as_os_str().to_owned();
        s.push(META_SUFFIX);
        PathBuf::from(s)
    }

    /// Stream `body` into `dest`, capping at `max` bytes. Returns the bytes
    /// written, or [`StorageError::TooLarge`] if the stream exceeds `max`. The
    /// partial file is removed on overflow.
    async fn stream_capped(dest: &Path, body: ByteReader, max: u64) -> StorageResult<u64> {
        let mut file = File::create(dest).await?;
        // `take(max + 1)` lets us *detect* an overflow: a stream longer than the
        // cap yields `max + 1` bytes, which we reject.
        let cap = max.saturating_add(1);
        let mut limited = body.take(cap);
        let written = match tokio::io::copy(&mut limited, &mut file).await {
            Ok(n) => n,
            Err(e) => {
                let _ = fs::remove_file(dest).await;
                return Err(e.into());
            }
        };
        file.flush().await?;
        if written > max {
            let _ = fs::remove_file(dest).await;
            return Err(StorageError::TooLarge { size: written, max });
        }
        Ok(written)
    }

    async fn write_sidecar(full: &Path, meta: &ObjectMeta) -> StorageResult<()> {
        let mut out = String::new();
        out.push_str("ct:");
        out.push_str(&meta.content_type);
        out.push('\n');
        for (k, v) in &meta.user_metadata {
            out.push_str("m:");
            out.push_str(k);
            out.push('=');
            out.push_str(v);
            out.push('\n');
        }
        fs::write(Self::sidecar_of(full), out).await?;
        Ok(())
    }

    async fn read_sidecar(full: &Path) -> (String, BTreeMap<String, String>) {
        let text = fs::read_to_string(Self::sidecar_of(full))
            .await
            .unwrap_or_default();
        let mut content_type = "application/octet-stream".to_string();
        let mut user_metadata = BTreeMap::new();
        for line in text.lines() {
            if let Some(ct) = line.strip_prefix("ct:") {
                content_type = ct.to_string();
            } else if let Some(kv) = line.strip_prefix("m:") {
                if let Some((k, v)) = kv.split_once('=') {
                    user_metadata.insert(k.to_string(), v.to_string());
                }
            }
        }
        (content_type, user_metadata)
    }

    /// Hash a file's bytes into a hex SHA-256 — the local adapter's ETag.
    async fn hash_file(path: &Path) -> StorageResult<String> {
        let mut file = File::open(path).await?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hex(&hasher.finalize()))
    }

    fn uploads_dir(&self) -> PathBuf {
        self.root.join(UPLOADS_DIR)
    }

    fn staging_of(&self, upload_id: &str) -> PathBuf {
        self.uploads_dir().join(upload_id)
    }

    /// Verify a presigned URL this store minted: the signature matches and the
    /// expiry is in the future at `now`. Returns the authorised verb and the
    /// resolved (tenant-prefixed) key. A local HTTP handler calls this before
    /// serving the bytes.
    ///
    /// # Errors
    ///
    /// [`StorageError::InvalidSignature`] for a tampered/malformed URL;
    /// [`StorageError::Expired`] once past the TTL.
    pub fn verify_presigned(
        &self,
        url: &str,
        now: SystemTime,
    ) -> StorageResult<(PresignVerb, String)> {
        let authority = format!("local://{}/", self.store_id);
        let rest = url
            .strip_prefix(&authority)
            .ok_or(StorageError::InvalidSignature)?;
        let (resolved_key, query) = rest.split_once('?').ok_or(StorageError::InvalidSignature)?;

        let mut verb = None;
        let mut exp = None;
        let mut sig = None;
        for pair in query.split('&') {
            match pair.split_once('=') {
                Some(("verb", v)) => verb = PresignVerb::parse(v),
                Some(("exp", v)) => exp = v.parse::<u64>().ok(),
                Some(("sig", v)) => sig = Some(v.to_string()),
                _ => {}
            }
        }
        let verb = verb.ok_or(StorageError::InvalidSignature)?;
        let exp = exp.ok_or(StorageError::InvalidSignature)?;
        let sig = sig.ok_or(StorageError::InvalidSignature)?;

        let expected = self.sign(verb, resolved_key, exp);
        if !constant_time_eq(expected.as_bytes(), sig.as_bytes()) {
            return Err(StorageError::InvalidSignature);
        }
        let now_unix = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        if now_unix > exp {
            return Err(StorageError::Expired);
        }
        Ok((verb, resolved_key.to_string()))
    }

    /// The hex SHA-256 over `key || verb || resolved_key || exp` — the presign
    /// signature. A keyed digest (the key is the first block), so a client cannot
    /// forge a URL without the signing key.
    fn sign(&self, verb: PresignVerb, resolved_key: &str, exp: u64) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.signing_key);
        hasher.update(b"\n");
        hasher.update(verb.as_str().as_bytes());
        hasher.update(b"\n");
        hasher.update(resolved_key.as_bytes());
        hasher.update(b"\n");
        hasher.update(exp.to_string().as_bytes());
        hex(&hasher.finalize())
    }
}

#[async_trait]
impl ObjectStore for LocalFsObjectStore {
    async fn put(
        &self,
        scope: Option<&Scope>,
        key: &str,
        body: ByteReader,
        meta: ObjectMeta,
    ) -> StorageResult<ObjectMeta> {
        self.limits.validate_meta(&meta)?;
        let resolved = resolve_key(scope, key)?;
        let full = self.confine(&resolved, true).await?;
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let tmp = Self::sidecar_of(&full).with_extension(format!("tmp.{seq}"));
        let written = Self::stream_capped(&tmp, body, self.limits.max_object_size).await?;
        fs::rename(&tmp, &full).await?;
        Self::write_sidecar(&full, &meta).await?;
        Ok(ObjectMeta {
            content_type: meta.content_type,
            content_length: written,
            user_metadata: meta.user_metadata,
        })
    }

    async fn get(
        &self,
        scope: Option<&Scope>,
        key: &str,
    ) -> StorageResult<(ObjectMeta, ByteReader)> {
        let resolved = resolve_key(scope, key)?;
        let full = self.confine(&resolved, false).await?;
        let file = File::open(&full).await?;
        let len = file.metadata().await?.len();
        let (content_type, user_metadata) = Self::read_sidecar(&full).await;
        let meta = ObjectMeta {
            content_type,
            content_length: len,
            user_metadata,
        };
        Ok((meta, Box::pin(file)))
    }

    async fn head(&self, scope: Option<&Scope>, key: &str) -> StorageResult<ObjectMeta> {
        let resolved = resolve_key(scope, key)?;
        let full = self.confine(&resolved, false).await?;
        let len = fs::metadata(&full).await?.len();
        let (content_type, user_metadata) = Self::read_sidecar(&full).await;
        Ok(ObjectMeta {
            content_type,
            content_length: len,
            user_metadata,
        })
    }

    async fn delete(&self, scope: Option<&Scope>, key: &str) -> StorageResult<()> {
        let resolved = resolve_key(scope, key)?;
        let full = self.confine(&resolved, false).await.ok();
        if let Some(full) = full {
            ignore_missing(fs::remove_file(&full).await)?;
            ignore_missing(fs::remove_file(Self::sidecar_of(&full)).await)?;
        }
        Ok(())
    }

    async fn list(&self, scope: Option<&Scope>, prefix: &str) -> StorageResult<Vec<ObjectRef>> {
        if !prefix.is_empty() {
            // A listing prefix is a key *fragment*; reject traversal but allow it
            // to be empty (list the whole tenant) or end at a path boundary.
            for component in prefix.split('/') {
                if component == ".." || component == "." {
                    return Err(StorageError::InvalidKey {
                        reason: "relative segment in prefix",
                    });
                }
            }
        }
        let scope_prefix = Self::scope_prefix(scope)?;
        let base = self.root.join(&scope_prefix);
        let mut out = Vec::new();
        let mut stack = vec![base.clone()];
        while let Some(dir) = stack.pop() {
            let mut entries = match fs::read_dir(&dir).await {
                Ok(e) => e,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                let file_type = entry.file_type().await?;
                if file_type.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.ends_with(META_SUFFIX) || name.contains(".tmp.") {
                    continue;
                }
                // Caller key = path relative to the tenant base, portable slashes.
                let Ok(rel) = path.strip_prefix(&base) else {
                    continue;
                };
                let key = rel.to_string_lossy().replace('\\', "/");
                if !prefix.is_empty() && !key.starts_with(prefix) {
                    continue;
                }
                let size = entry.metadata().await?.len();
                out.push(ObjectRef { key, size });
            }
        }
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }

    async fn presign(
        &self,
        scope: Option<&Scope>,
        key: &str,
        verb: PresignVerb,
        ttl: Duration,
    ) -> StorageResult<PresignedUrl> {
        let resolved = resolve_key(scope, key)?;
        let now = self
            .clock
            .now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let exp = now.saturating_add(ttl.as_secs());
        let sig = self.sign(verb, &resolved, exp);
        let url = format!(
            "local://{}/{}?verb={}&exp={}&sig={}",
            self.store_id,
            resolved,
            verb.as_str(),
            exp,
            sig
        );
        Ok(PresignedUrl::new(url))
    }

    async fn create_multipart(
        &self,
        scope: Option<&Scope>,
        key: &str,
        meta: ObjectMeta,
    ) -> StorageResult<MultipartUpload> {
        self.limits.validate_meta(&meta)?;
        let resolved = resolve_key(scope, key)?;
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let upload_id = format!("{seq}");
        fs::create_dir_all(self.staging_of(&upload_id)).await?;
        Ok(MultipartUpload {
            upload_id,
            resolved_key: resolved,
            meta,
        })
    }

    async fn upload_part(
        &self,
        upload: &MultipartUpload,
        part_number: u32,
        body: ByteReader,
    ) -> StorageResult<PartTag> {
        let staging = self.staging_of(&upload.upload_id);
        if !fs::try_exists(&staging).await? {
            return Err(StorageError::NoSuchUpload);
        }
        let part_path = staging.join(format!("part-{part_number:08}"));
        // A single part is still capped by the object limit (a defensive bound).
        Self::stream_capped(&part_path, body, self.limits.max_object_size).await?;
        let etag = Self::hash_file(&part_path).await?;
        Ok(PartTag { part_number, etag })
    }

    async fn complete_multipart(
        &self,
        upload: MultipartUpload,
        parts: Vec<PartTag>,
    ) -> StorageResult<ObjectMeta> {
        let staging = self.staging_of(&upload.upload_id);
        if !fs::try_exists(&staging).await? {
            return Err(StorageError::NoSuchUpload);
        }
        let full = self.confine(&upload.resolved_key, true).await?;
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let tmp = Self::sidecar_of(&full).with_extension(format!("tmp.{seq}"));
        let mut dest = File::create(&tmp).await?;
        let mut total: u64 = 0;
        for part in &parts {
            let part_path = staging.join(format!("part-{:08}", part.part_number));
            let mut src = File::open(&part_path)
                .await
                .map_err(|_| StorageError::NoSuchUpload)?;
            let copied = tokio::io::copy(&mut src, &mut dest).await?;
            total = total.saturating_add(copied);
            if total > self.limits.max_object_size {
                let _ = fs::remove_file(&tmp).await;
                let _ = fs::remove_dir_all(&staging).await;
                return Err(StorageError::TooLarge {
                    size: total,
                    max: self.limits.max_object_size,
                });
            }
        }
        dest.flush().await?;
        fs::rename(&tmp, &full).await?;
        let meta = ObjectMeta {
            content_type: upload.meta.content_type,
            content_length: total,
            user_metadata: upload.meta.user_metadata,
        };
        Self::write_sidecar(&full, &meta).await?;
        let _ = fs::remove_dir_all(&staging).await;
        Ok(meta)
    }

    async fn abort_multipart(&self, upload: MultipartUpload) -> StorageResult<()> {
        ignore_missing(fs::remove_dir_all(self.staging_of(&upload.upload_id)).await)?;
        Ok(())
    }
}

/// Map a "not found" I/O result to `Ok(())` so delete/abort are idempotent.
fn ignore_missing(result: std::io::Result<()>) -> StorageResult<()> {
    match result {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Lowercase hex of a byte slice (the digest rendering used for ETags and sigs).
fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}

/// Length-independent-return byte comparison for signatures: no early-exit on
/// the first mismatch, so a verifier leaks no timing signal about the digest.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
