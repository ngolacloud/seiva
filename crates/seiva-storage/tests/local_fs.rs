//! End-to-end proofs for the default `LocalFsObjectStore`: streaming round-trip,
//! tenant isolation, fail-closed scope, size/validation, listing, multipart,
//! presigned URLs and symlink-escape confinement.

use std::io::Cursor;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use seiva_ports::{Clock, Scope};
use seiva_storage::{
    byte_reader, LocalFsObjectStore, ObjectMeta, ObjectStore, PresignVerb, StorageError,
    StoreLimits,
};
use tokio::io::AsyncReadExt;

static SEQ: AtomicU32 = AtomicU32::new(0);

/// A fresh, unique temp directory for one test's store root.
fn temp_root() -> std::path::PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("seiva-storage-{}-{}", std::process::id(), n))
}

fn tenant(id: &str) -> Scope {
    Scope::tenant(id).expect("non-empty tenant")
}

fn reader(bytes: &[u8]) -> seiva_storage::ByteReader {
    byte_reader(Cursor::new(bytes.to_vec()))
}

async fn read_all(mut r: seiva_storage::ByteReader) -> Vec<u8> {
    let mut buf = Vec::new();
    r.read_to_end(&mut buf).await.expect("read body");
    buf
}

#[tokio::test]
async fn put_then_get_round_trips_streaming() {
    let store = LocalFsObjectStore::new(temp_root()).await.unwrap();
    let t = tenant("acme");
    let payload = b"a recording blob";

    let put = store
        .put(
            Some(&t),
            "recordings/2026/a.mp4",
            reader(payload),
            ObjectMeta::new("video/mp4", payload.len() as u64).with_meta("id", "rec-1"),
        )
        .await
        .unwrap();
    assert_eq!(put.content_length, payload.len() as u64);

    let (meta, body) = store.get(Some(&t), "recordings/2026/a.mp4").await.unwrap();
    assert_eq!(meta.content_type, "video/mp4");
    assert_eq!(meta.content_length, payload.len() as u64);
    assert_eq!(
        meta.user_metadata.get("id").map(String::as_str),
        Some("rec-1")
    );
    assert_eq!(read_all(body).await, payload);
}

#[tokio::test]
async fn tenants_are_isolated_by_construction() {
    let store = LocalFsObjectStore::new(temp_root()).await.unwrap();
    store
        .put(
            Some(&tenant("a")),
            "doc.txt",
            reader(b"secret"),
            ObjectMeta::new("text/plain", 6),
        )
        .await
        .unwrap();

    // Tenant B asks for the same caller key → a different prefix → not found.
    // (`get`'s Ok carries a non-Debug reader, so match instead of `unwrap_err`.)
    match store.get(Some(&tenant("b")), "doc.txt").await {
        Err(StorageError::NotFound) => {}
        other => panic!("expected NotFound, got {:?}", other.err()),
    }
}

#[tokio::test]
async fn missing_scope_fails_closed() {
    let store = LocalFsObjectStore::new(temp_root()).await.unwrap();
    let err = store
        .put(
            None,
            "x.txt",
            reader(b"hi"),
            ObjectMeta::new("text/plain", 2),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, StorageError::MissingScope));
}

#[tokio::test]
async fn oversize_object_is_rejected_and_not_persisted() {
    let store = LocalFsObjectStore::new(temp_root())
        .await
        .unwrap()
        .with_limits(StoreLimits {
            max_object_size: 4,
            allowed_content_types: None,
        });
    let t = tenant("acme");

    // Declared length is within the cap, but the stream delivers more: the
    // post-stream re-check must still catch it.
    let err = store
        .put(
            Some(&t),
            "big.bin",
            reader(b"0123456789"),
            ObjectMeta::new("application/octet-stream", 3),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, StorageError::TooLarge { .. }), "got {err:?}");

    // And nothing was left behind.
    assert!(matches!(
        store.head(Some(&t), "big.bin").await.unwrap_err(),
        StorageError::NotFound
    ));
}

#[tokio::test]
async fn head_then_delete_is_idempotent() {
    let store = LocalFsObjectStore::new(temp_root()).await.unwrap();
    let t = tenant("acme");
    store
        .put(
            Some(&t),
            "a.txt",
            reader(b"hello"),
            ObjectMeta::new("text/plain", 5),
        )
        .await
        .unwrap();

    assert_eq!(
        store.head(Some(&t), "a.txt").await.unwrap().content_length,
        5
    );
    store.delete(Some(&t), "a.txt").await.unwrap();
    assert!(matches!(
        store.head(Some(&t), "a.txt").await.unwrap_err(),
        StorageError::NotFound
    ));
    // Deleting an absent key is a no-op, not an error.
    store.delete(Some(&t), "a.txt").await.unwrap();
}

#[tokio::test]
async fn list_returns_caller_keys_without_tenant_prefix() {
    let store = LocalFsObjectStore::new(temp_root()).await.unwrap();
    let t = tenant("acme");
    for key in ["docs/a.txt", "docs/b.txt", "images/c.png"] {
        store
            .put(
                Some(&t),
                key,
                reader(b"x"),
                ObjectMeta::new("text/plain", 1),
            )
            .await
            .unwrap();
    }

    let all = store.list(Some(&t), "").await.unwrap();
    let keys: Vec<&str> = all.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(keys, ["docs/a.txt", "docs/b.txt", "images/c.png"]);

    let docs = store.list(Some(&t), "docs/").await.unwrap();
    let docs_keys: Vec<&str> = docs.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(docs_keys, ["docs/a.txt", "docs/b.txt"]);
    // Sidecar metadata files never surface in a listing.
    assert!(all.iter().all(|r| !r.key.ends_with(".seivameta")));
}

#[tokio::test]
async fn multipart_assembles_parts_in_order() {
    let store = LocalFsObjectStore::new(temp_root()).await.unwrap();
    let t = tenant("acme");

    let upload = store
        .create_multipart(
            Some(&t),
            "big/file.bin",
            ObjectMeta::new("application/octet-stream", 0),
        )
        .await
        .unwrap();
    let p1 = store
        .upload_part(&upload, 1, reader(b"AAAA"))
        .await
        .unwrap();
    let p2 = store
        .upload_part(&upload, 2, reader(b"BBBB"))
        .await
        .unwrap();
    let p3 = store.upload_part(&upload, 3, reader(b"CC")).await.unwrap();

    let meta = store
        .complete_multipart(upload, vec![p1, p2, p3])
        .await
        .unwrap();
    assert_eq!(meta.content_length, 10);

    let (_m, body) = store.get(Some(&t), "big/file.bin").await.unwrap();
    assert_eq!(read_all(body).await, b"AAAABBBBCC");
}

#[tokio::test]
async fn multipart_abort_discards_staging() {
    let store = LocalFsObjectStore::new(temp_root()).await.unwrap();
    let t = tenant("acme");
    let upload = store
        .create_multipart(
            Some(&t),
            "x.bin",
            ObjectMeta::new("application/octet-stream", 0),
        )
        .await
        .unwrap();
    store
        .upload_part(&upload, 1, reader(b"AAAA"))
        .await
        .unwrap();
    store.abort_multipart(upload.clone()).await.unwrap();

    // The object was never finalised, and completing an aborted upload fails.
    assert!(matches!(
        store.head(Some(&t), "x.bin").await.unwrap_err(),
        StorageError::NotFound
    ));
    assert!(matches!(
        store.complete_multipart(upload, vec![]).await.unwrap_err(),
        StorageError::NoSuchUpload
    ));
}

/// A clock pinned to a fixed instant so presign expiry is deterministic.
#[derive(Clone)]
struct FixedClock(SystemTime);
impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

#[tokio::test]
async fn presigned_url_signs_and_verifies_then_expires() {
    let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    let store = LocalFsObjectStore::new(temp_root())
        .await
        .unwrap()
        .with_signing_key("a-strong-signing-secret")
        .with_clock(Arc::new(FixedClock(t0)));
    let t = tenant("acme");

    let url = store
        .presign(
            Some(&t),
            "recordings/a.mp4",
            PresignVerb::Get,
            Duration::from_secs(60),
        )
        .await
        .unwrap();

    // Valid within the TTL: verification yields the verb and the resolved key.
    let (verb, resolved) = store
        .verify_presigned(url.as_str(), t0)
        .expect("presigned url should verify within ttl");
    assert_eq!(verb, PresignVerb::Get);
    assert_eq!(resolved, "tenant/acme/recordings/a.mp4");

    // Past the TTL → expired.
    let later = t0 + Duration::from_secs(61);
    assert!(matches!(
        store.verify_presigned(url.as_str(), later),
        Err(StorageError::Expired)
    ));

    // A tampered signature → invalid.
    let tampered = url.as_str().replace("sig=", "sig=deadbeef");
    assert!(matches!(
        store.verify_presigned(&tampered, t0),
        Err(StorageError::InvalidSignature)
    ));
}

#[tokio::test]
async fn symlink_escape_is_blocked() {
    let root = temp_root();
    let store = LocalFsObjectStore::new(&root).await.unwrap();
    let t = tenant("acme");
    // Materialise the tenant directory with one real object.
    store
        .put(
            Some(&t),
            "real.txt",
            reader(b"ok"),
            ObjectMeta::new("text/plain", 2),
        )
        .await
        .unwrap();

    // Plant a symlink inside the tenant tree pointing outside the store root.
    let outside = temp_root();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), b"TOP SECRET").unwrap();
    let canon_root = std::fs::canonicalize(&root).unwrap();
    let link = canon_root.join("tenant/acme/escape");
    std::os::unix::fs::symlink(std::fs::canonicalize(&outside).unwrap(), &link).unwrap();

    // A key whose directory component is the planted symlink must be refused: its
    // canonical parent falls outside the store root.
    match store.get(Some(&t), "escape/secret.txt").await {
        Err(StorageError::InvalidKey { .. }) => {}
        other => panic!("expected InvalidKey, got {:?}", other.err()),
    }
}
