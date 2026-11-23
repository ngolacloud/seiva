//! Delonix Seiva — a pluggable, **tenant-aware object store** (ADR 0032): the
//! framework's blob capability.
//!
//! Seiva's data layer stores *records*; a platform also needs to store *blobs* —
//! recordings, exports, uploaded documents, avatars, generated reports. The
//! relational store is the wrong place for them, and a naïve `put(key, Vec<u8>)`
//! helper is a trap (a 2 GB recording buffered whole in process memory). This
//! crate is the object capability, built once, correctly: streaming, tenant-safe
//! and fail-closed from the first line.
//!
//! # The shape
//!
//! - [`ObjectStore`] — the port: `put`/`get` stream an [`ByteReader`] both ways
//!   (never materialising a whole object), plus `head`/`delete`/`list`, presigned
//!   URLs ([`ObjectStore::presign`]) and a multipart lifecycle for large objects.
//! - [`resolve_key`] — the security heart: every caller key is validated
//!   traversal-free and resolved under a **per-tenant prefix** from the ambient
//!   [`Scope`](seiva_ports::Scope); a `None` scope **fails closed**
//!   ([`StorageError::MissingScope`]), never a shared-root write.
//! - [`LocalFsObjectStore`] — the default adapter: a directory tree confined to
//!   its tenant prefix (rejecting `..`, absolute paths and symlink escape), so it
//!   runs offline in dev and test with no vendor SDK.
//! - [`ResilientStore`] — decorates any store with `seiva-resilience` retry,
//!   timeout and an optional circuit breaker, reused rather than reinvented.
//!
//! # Zero-cost when unused, DORM-free
//!
//! The library links **no runtime** (`tokio` enters with no executor feature,
//! only `io-util` + `fs` for the local adapter) and **no data layer**. A vendor
//! S3 adapter belongs behind a feature so the core links no AWS SDK; a service
//! that stores nothing pays nothing.
//!
//! # Safety model (non-negotiable, ADR 0032)
//!
//! - **Tenant prefix in the mechanism** — [`resolve_key`] keys every blob under
//!   its tenant; no product can forget it.
//! - **Fail-closed** — a missing [`Scope`](seiva_ports::Scope) is an error, not a
//!   fall-through to a shared root (ADR 0019).
//! - **Path confinement** — the local adapter canonicalises and confines every
//!   path under the store root, blocking traversal and symlink escape.
//! - **Validated writes** — content-type and size are checked before a write
//!   commits, and the streamed byte count is re-checked against the cap.
//! - **Short-TTL presigned URLs** — a signed, time-bounded, verb/key-scoped URL
//!   delegates the transfer to the store instead of proxying bytes through the app.

#![forbid(unsafe_code)]

mod error;
mod key;
mod local;
mod meta;
mod resilient;
mod store;

pub use error::{StorageError, StorageResult};
pub use key::{resolve_key, validate_key, PresignVerb};
pub use local::{LocalFsObjectStore, SystemClock};
pub use meta::{ObjectMeta, ObjectRef, PresignedUrl, StoreLimits};
pub use resilient::ResilientStore;
pub use store::{byte_reader, ByteReader, MultipartUpload, ObjectStore, PartTag};
