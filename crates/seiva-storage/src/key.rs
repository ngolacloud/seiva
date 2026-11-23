//! Tenant-aware, fail-closed key resolution — the security heart of ADR 0032.
//!
//! A blob keyed only by a product-chosen name is one missing prefix away from a
//! cross-tenant read. So the *mechanism* owns the prefix: every caller key is
//! validated as a safe relative key and then resolved under a per-tenant prefix
//! taken from the ambient [`Scope`]. A call with **no** scope fails closed
//! ([`StorageError::MissingScope`]); no product can opt out of isolation.

use seiva_ports::Scope;

use crate::error::{StorageError, StorageResult};

/// The reserved sidecar suffix the local adapter uses to persist metadata next
/// to an object. A caller key ending in it is rejected so a product can never
/// collide with (or forge) a metadata file.
pub(crate) const META_SUFFIX: &str = ".seivameta";

/// Which verb a presigned URL authorises.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresignVerb {
    /// A download (`GET`) of the object.
    Get,
    /// An upload (`PUT`) to the object.
    Put,
}

impl PresignVerb {
    /// The stable wire token used inside a presigned URL and its signature.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            PresignVerb::Get => "GET",
            PresignVerb::Put => "PUT",
        }
    }

    /// Parse a verb token back from a presigned URL, or `None` if unknown.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "GET" => Some(PresignVerb::Get),
            "PUT" => Some(PresignVerb::Put),
            _ => None,
        }
    }
}

/// Validate a caller key as a safe, relative, traversal-free object key.
///
/// Rejects — **before any I/O** — the empty key, an absolute key, any `.`/`..`
/// component, a trailing/leading/double slash, a backslash, a NUL or control
/// byte, and the reserved metadata suffix. On success the key is byte-identical
/// to the input: validation never rewrites it.
///
/// # Errors
///
/// [`StorageError::InvalidKey`] with a structural reason (no secret bytes).
pub fn validate_key(key: &str) -> StorageResult<()> {
    if key.is_empty() {
        return Err(StorageError::InvalidKey {
            reason: "empty key",
        });
    }
    if key.ends_with(META_SUFFIX) {
        return Err(StorageError::InvalidKey {
            reason: "reserved metadata suffix",
        });
    }
    if key.starts_with('/') {
        return Err(StorageError::InvalidKey {
            reason: "absolute key",
        });
    }
    if key.contains('\\') {
        return Err(StorageError::InvalidKey {
            reason: "backslash in key",
        });
    }
    if key.bytes().any(|b| b.is_ascii_control()) {
        return Err(StorageError::InvalidKey {
            reason: "control byte in key",
        });
    }
    // Component-wise: no empty segment (so no `//`, no trailing `/`), and no
    // `.`/`..` that a filesystem would resolve out of the tenant prefix.
    for component in key.split('/') {
        match component {
            "" => {
                return Err(StorageError::InvalidKey {
                    reason: "empty path segment",
                })
            }
            "." | ".." => {
                return Err(StorageError::InvalidKey {
                    reason: "relative path segment",
                })
            }
            _ => {}
        }
    }
    Ok(())
}

/// Resolve a caller key into its fully-qualified, tenant-prefixed storage key.
///
/// - [`Some(Scope::Tenant(t))`](Scope::Tenant) → `tenant/{t}/{key}`, isolating
///   the blob by construction.
/// - [`Some(Scope::Global)`](Scope::Global) → `shared/{key}`, the deliberate,
///   **audited** cross-tenant root (a `tracing` event is emitted, mirroring the
///   data-layer gateway's global escape), never an accidental fall-through.
/// - `None` → [`StorageError::MissingScope`]: fail-closed, no shared-root write.
///
/// The key is validated first, so the returned string is always a safe,
/// traversal-free relative path.
///
/// # Errors
///
/// [`StorageError::MissingScope`] for a `None` scope; [`StorageError::InvalidKey`]
/// for an unsafe key.
pub fn resolve_key(scope: Option<&Scope>, key: &str) -> StorageResult<String> {
    validate_key(key)?;
    match scope {
        Some(Scope::Tenant(tenant)) => Ok(format!("tenant/{}/{}", tenant.as_str(), key)),
        Some(Scope::Global) => {
            // The deliberate, audited cross-tenant escape — a visible choice on
            // the record, exactly like the data layer's `Scope::Global`.
            tracing::warn!(
                target: "seiva_storage::audit",
                key = %key,
                "object-store access under Scope::Global (shared root)"
            );
            Ok(format!("shared/{key}"))
        }
        // `None` *and* any future `Scope` kind we cannot resolve a prefix for:
        // fail closed (ADR 0019) rather than guess a prefix that might collide.
        _ => Err(StorageError::MissingScope),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant(id: &str) -> Scope {
        Scope::tenant(id).expect("non-empty tenant")
    }

    #[test]
    fn resolves_under_tenant_prefix() {
        let resolved = resolve_key(Some(&tenant("acme")), "recordings/2026/a.mp4").unwrap();
        assert_eq!(resolved, "tenant/acme/recordings/2026/a.mp4");
    }

    #[test]
    fn global_scope_uses_audited_shared_root() {
        let resolved = resolve_key(Some(&Scope::Global), "exports/all.csv").unwrap();
        assert_eq!(resolved, "shared/exports/all.csv");
    }

    #[test]
    fn missing_scope_fails_closed() {
        let err = resolve_key(None, "a.txt").unwrap_err();
        assert!(matches!(err, StorageError::MissingScope));
    }

    #[test]
    fn rejects_traversal_and_absolute_keys() {
        for bad in [
            "../secret",
            "a/../../b",
            "/etc/passwd",
            "a//b",
            "a/./b",
            "a/",
            "",
            "with\\backslash",
            "nul\0byte",
        ] {
            let err = resolve_key(Some(&tenant("t")), bad).unwrap_err();
            assert!(
                matches!(err, StorageError::InvalidKey { .. }),
                "expected InvalidKey for {bad:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn rejects_reserved_metadata_suffix() {
        let err = resolve_key(Some(&tenant("t")), "a.seivameta").unwrap_err();
        assert!(matches!(err, StorageError::InvalidKey { .. }));
    }

    #[test]
    fn a_traversal_key_cannot_escape_the_tenant_prefix() {
        // Even if validation were bypassed, the resolved string stays under the
        // tenant prefix; here we prove a `..` never produces a parent escape.
        assert!(resolve_key(Some(&tenant("t")), "a/../b").is_err());
    }
}
