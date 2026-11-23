//! The value types the port moves: object metadata, a listing entry, the store
//! limits a `put` is validated against, and the presigned-URL newtype.

use std::collections::BTreeMap;

use crate::error::{StorageError, StorageResult};

/// Content-type, length and an optional product-set metadata map for one object.
///
/// On `put` the caller supplies the declared `content_type` and
/// `content_length`; the store validates both against its [`StoreLimits`] before
/// a byte is written and returns a meta carrying the **actual** streamed length.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectMeta {
    /// The MIME type, e.g. `video/mp4`. Must be non-empty and (if the store
    /// configures an allowlist) permitted.
    pub content_type: String,
    /// The object size in bytes. Declared on `put`; the returned meta carries the
    /// bytes actually streamed.
    pub content_length: u64,
    /// Opaque product metadata (e.g. `recording-id`). Keys/values must be free of
    /// newlines so the local adapter can persist them line-by-line.
    pub user_metadata: BTreeMap<String, String>,
}

impl ObjectMeta {
    /// A meta with just a content-type and length and no user metadata.
    #[must_use]
    pub fn new(content_type: impl Into<String>, content_length: u64) -> Self {
        Self {
            content_type: content_type.into(),
            content_length,
            user_metadata: BTreeMap::new(),
        }
    }

    /// Attach one user-metadata entry, builder-style.
    #[must_use]
    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.user_metadata.insert(key.into(), value.into());
        self
    }
}

/// One entry returned by [`list`](crate::ObjectStore::list): the caller-visible
/// key (the tenant prefix stripped back off) and its size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRef {
    /// The key as the product sees it, without the tenant prefix.
    pub key: String,
    /// The object size in bytes.
    pub size: u64,
}

/// The limits a `put` (or `create_multipart`) is validated against before any
/// bytes are written — the mechanism's bound, not a per-call option.
#[derive(Clone, Debug)]
pub struct StoreLimits {
    /// The largest object the store accepts, in bytes. Enforced on the declared
    /// length **and** on the streamed bytes.
    pub max_object_size: u64,
    /// An optional content-type allowlist. `None` permits any non-empty type;
    /// `Some(set)` rejects anything outside it.
    pub allowed_content_types: Option<std::collections::BTreeSet<String>>,
}

impl Default for StoreLimits {
    /// 5 GiB and any non-empty content-type — a generous default a deployment
    /// tightens, not a policy baked into the mechanism.
    fn default() -> Self {
        Self {
            max_object_size: 5 * 1024 * 1024 * 1024,
            allowed_content_types: None,
        }
    }
}

impl StoreLimits {
    /// Validate a declared meta against these limits **before** a write commits.
    ///
    /// # Errors
    ///
    /// - [`StorageError::UnsupportedContentType`] for an empty or disallowed type;
    /// - [`StorageError::TooLarge`] if the declared length exceeds the cap;
    /// - [`StorageError::InvalidKey`] if a user-metadata key/value carries a
    ///   newline (it would corrupt the local adapter's line-based sidecar).
    pub fn validate_meta(&self, meta: &ObjectMeta) -> StorageResult<()> {
        if meta.content_type.trim().is_empty() {
            return Err(StorageError::UnsupportedContentType {
                content_type: meta.content_type.clone(),
            });
        }
        if let Some(allowed) = &self.allowed_content_types {
            if !allowed.contains(&meta.content_type) {
                return Err(StorageError::UnsupportedContentType {
                    content_type: meta.content_type.clone(),
                });
            }
        }
        if meta.content_length > self.max_object_size {
            return Err(StorageError::TooLarge {
                size: meta.content_length,
                max: self.max_object_size,
            });
        }
        for (k, v) in &meta.user_metadata {
            if k.contains(['\n', '\r']) || v.contains(['\n', '\r']) {
                return Err(StorageError::InvalidKey {
                    reason: "newline in user metadata",
                });
            }
        }
        Ok(())
    }
}

/// A short-TTL signed URL that delegates the actual transfer to the store, so
/// bytes flow client↔storage directly and the app only authorises and signs.
///
/// A newtype over the URL string rather than a dependency on a URL crate: the
/// port stays dependency-light and both the local and S3 adapters mint the same
/// shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresignedUrl(String);

impl PresignedUrl {
    /// Wrap a URL string a presigning adapter produced.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into())
    }

    /// The URL as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the newtype, returning the owned URL string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for PresignedUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_disallowed_content_type() {
        let mut allowed = std::collections::BTreeSet::new();
        allowed.insert("image/png".to_string());
        let limits = StoreLimits {
            max_object_size: 1024,
            allowed_content_types: Some(allowed),
        };
        assert!(limits.validate_meta(&ObjectMeta::new("", 1)).is_err());
        assert!(limits
            .validate_meta(&ObjectMeta::new("image/gif", 1))
            .is_err());
        assert!(limits
            .validate_meta(&ObjectMeta::new("image/png", 1))
            .is_ok());
    }

    #[test]
    fn rejects_declared_oversize() {
        let limits = StoreLimits {
            max_object_size: 10,
            allowed_content_types: None,
        };
        let err = limits
            .validate_meta(&ObjectMeta::new("text/plain", 11))
            .unwrap_err();
        assert!(matches!(err, StorageError::TooLarge { size: 11, max: 10 }));
    }

    #[test]
    fn rejects_newline_in_user_metadata() {
        let limits = StoreLimits::default();
        let meta = ObjectMeta::new("text/plain", 1).with_meta("k", "line1\nline2");
        assert!(limits.validate_meta(&meta).is_err());
    }
}
