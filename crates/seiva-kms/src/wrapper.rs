//! The `KeyWrapper` port: the guardian of the key-encryption key.

use crate::{DataKey, KmsError};

/// A data key fresh from the guardian: the key itself (kept in memory only) and
/// its wrapped form (the only thing ever stored).
#[derive(Debug)]
pub struct GeneratedKey {
    /// The plain data key.
    pub plain: DataKey,
    /// The guardian's wrapped form (`vault:v1:…`, `local:<kek-fp>:…`).
    pub wrapped: String,
}

/// The guardian of a key-encryption key (KEK): it makes data keys and wraps and
/// unwraps them, and the KEK never leaves it.
///
/// Adapters: [`LocalKek`](crate::LocalKek) (feature `local`) and
/// `VaultTransit` (feature `vault`). AWS KMS, Google Cloud
/// KMS and Azure Key Vault come later behind the same port (ADR 0049).
///
/// Every method fails with [`KmsError::Unavailable`] when the guardian cannot
/// be reached (worth retrying) and [`KmsError::Refused`] when it says no.
#[async_trait::async_trait]
pub trait KeyWrapper: Send + Sync {
    /// The adapter: `local`, `vault`.
    fn provider(&self) -> &'static str;

    /// The KEK as configured: the transit key's name, the local key file's
    /// name. Recorded with every wrapped data key; a ring wrapped by another KEK
    /// is refused rather than silently mixed.
    fn kek_id(&self) -> &str;

    /// A new random data key and its wrapped form.
    async fn generate(&self) -> Result<GeneratedKey, KmsError>;

    /// Opens a data key that KEK `kek_id` wrapped.
    async fn unwrap(&self, kek_id: &str, wrapped: &str) -> Result<DataKey, KmsError>;

    /// The data key wrapped with the newest version of the KEK, when `wrapped`
    /// used an older one; `None` when it is already current. Called after the
    /// guardian rotated the KEK, so old versions can later be retired there.
    async fn rewrap(
        &self,
        kek_id: &str,
        wrapped: &str,
        plain: &DataKey,
    ) -> Result<Option<String>, KmsError>;

    /// The guardian is reachable and this process is allowed to use it.
    async fn health(&self) -> Result<(), KmsError>;
}
