//! The one failure type every adapter maps into, HTTP- and vendor-neutral.

use seiva_ports::BoxError;

/// Result returned by every [`ObjectStore`](crate::ObjectStore) method.
pub type StorageResult<T> = Result<T, StorageError>;

/// Why an object-store operation was refused or failed.
///
/// Free of any HTTP or vendor type so the port stays transport-neutral; a
/// transport maps each variant to a status (`403`/`400`/`404`/`413`/`502`).
/// `#[non_exhaustive]`: a consumer matches with a `_` arm so a future variant is
/// a non-breaking addition.
#[derive(Debug)]
#[non_exhaustive]
pub enum StorageError {
    /// No ambient [`Scope`](seiva_ports::Scope) was supplied: the call **fails
    /// closed** (ADR 0019) rather than falling back to a shared root. The
    /// deny-by-default answer for a tenant-keyed store.
    MissingScope,
    /// The caller key is not a safe relative key: empty, absolute, contains a
    /// `..`/`.` component, a NUL or control byte, a backslash, or the reserved
    /// metadata suffix. Blocks path traversal before any I/O (maps to `400`).
    InvalidKey {
        /// Why the key was rejected (no secret bytes, just the structural reason).
        reason: &'static str,
    },
    /// The requested object does not exist (maps to `404`).
    NotFound,
    /// The object (declared or streamed) exceeds the store's size limit (maps to
    /// `413`). Enforced before *and* after streaming so a lying length cannot
    /// slip past.
    TooLarge {
        /// The offending size in bytes (the declared length, or the bytes seen).
        size: u64,
        /// The configured maximum in bytes.
        max: u64,
    },
    /// The object's content-type is empty or not in the configured allowlist
    /// (maps to `400`).
    UnsupportedContentType {
        /// The rejected content-type.
        content_type: String,
    },
    /// A presigned URL failed verification: tampered, malformed, or for the
    /// wrong verb/key (maps to `403`).
    InvalidSignature,
    /// A presigned URL is past its expiry (maps to `403`).
    Expired,
    /// A referenced multipart upload is unknown or already finished (maps to `409`).
    NoSuchUpload,
    /// An adapter/backend error (an I/O error, an S3 error) mapped at the
    /// boundary. The source is boxed so this crate carries no vendor error type
    /// (maps to `502`).
    Backend(BoxError),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::MissingScope => {
                f.write_str("object-store call has no tenant scope (fail-closed)")
            }
            StorageError::InvalidKey { reason } => write!(f, "invalid object key: {reason}"),
            StorageError::NotFound => f.write_str("object not found"),
            StorageError::TooLarge { size, max } => {
                write!(f, "object too large: {size} bytes exceeds max {max}")
            }
            StorageError::UnsupportedContentType { content_type } => {
                write!(f, "unsupported content-type: {content_type:?}")
            }
            StorageError::InvalidSignature => f.write_str("invalid presigned-url signature"),
            StorageError::Expired => f.write_str("presigned url expired"),
            StorageError::NoSuchUpload => f.write_str("no such multipart upload"),
            StorageError::Backend(e) => write!(f, "object-store backend error: {e}"),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StorageError::Backend(e) => Some(&**e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for StorageError {
    /// A missing file maps to [`NotFound`](StorageError::NotFound); every other
    /// I/O error is a [`Backend`](StorageError::Backend) failure.
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::NotFound {
            StorageError::NotFound
        } else {
            StorageError::Backend(Box::new(e))
        }
    }
}
