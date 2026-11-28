//! The crate's one error type.

/// What can go wrong wrapping, sealing or keeping keys.
///
/// No variant ever carries key material or plaintext. [`KmsError::Decrypt`] does
/// not say *why* a blob did not open (wrong key, wrong AAD, tampered bytes):
/// telling them apart would help an attacker, not the operator.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum KmsError {
    /// The guardian did not answer, or answered "try later" (5xx, 429, sealed).
    /// Worth retrying: [`KmsError::is_transient`] is true.
    #[error("key guardian unavailable: {0}")]
    Unavailable(String),
    /// The guardian refused (authentication, permission, unknown key).
    #[error("key guardian refused: {0}")]
    Refused(String),
    /// The guardian answered something this crate cannot use.
    #[error("key guardian returned an invalid answer: {0}")]
    Malformed(String),
    /// The configuration is wrong (a missing file, a key of the wrong length, a
    /// ring wrapped by another guardian than the configured one).
    #[error("key configuration: {0}")]
    Config(String),
    /// A blob is sealed with a key the ring does not hold (retired, or rotated by
    /// another process moments ago: re-read the ring and retry once).
    #[error("sealed with key '{0}', which this key ring does not hold")]
    UnknownKey(String),
    /// A blob is not in the expected format (prefix, key id, base64, length).
    #[error("sealed data is malformed: {0}")]
    BadSealed(String),
    /// A blob did not open: wrong key, wrong associated data, or altered bytes.
    #[error("sealed data does not open with this key and associated data")]
    Decrypt,
    /// The stored key ring cannot be read or is inconsistent. Fails closed: no
    /// key from a ring that does not parse is ever used.
    #[error("key ring is corrupted: {0}")]
    Ring(String),
    /// Somebody else changed the stored ring since it was read (optimistic
    /// concurrency): re-read it.
    #[error("the key ring changed since it was read")]
    Conflict,
    /// The caller's [`RingStore`](crate::RingStore) or
    /// [`Reseal`](crate::Reseal) failed.
    #[error("key ring storage: {0}")]
    Store(String),
    /// The operating system's random generator failed.
    #[error("the system random generator failed")]
    Random,
}

impl KmsError {
    /// An unreachable or busy guardian: start-up keeps retrying it until its
    /// deadline. Every other error is final.
    pub fn is_transient(&self) -> bool {
        matches!(self, KmsError::Unavailable(_))
    }
}
