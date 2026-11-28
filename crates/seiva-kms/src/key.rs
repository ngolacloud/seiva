//! Key material: 32 bytes that are wiped when dropped and never printed.

use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::KmsError;

/// A 32-byte symmetric key (a data key, or a local KEK).
///
/// Wiped from memory when dropped; `Debug` prints `DataKey(..)`, never the bytes.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct DataKey([u8; 32]);

impl DataKey {
    /// Wraps 32 bytes (the caller should wipe its own copy).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// A slice that must be exactly 32 bytes long; `who` names the source in
    /// the error ("Vault", "line 2 of /run/kek").
    pub fn from_slice(bytes: &[u8], who: &str) -> Result<Self, KmsError> {
        let arr: [u8; 32] = bytes.try_into().map_err(|_| {
            KmsError::Malformed(format!(
                "{who}: a key must be 32 bytes, got {}",
                bytes.len()
            ))
        })?;
        Ok(Self(arr))
    }

    /// 32 random bytes from the operating system.
    pub fn random() -> Result<Self, KmsError> {
        let mut k = [0u8; 32];
        random_bytes(&mut k)?;
        let key = Self(k);
        k.zeroize();
        Ok(key)
    }

    /// The key bytes, for a cipher.
    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for DataKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DataKey(..)")
    }
}

/// Fills `buf` from the operating system's random generator.
pub(crate) fn random_bytes(buf: &mut [u8]) -> Result<(), KmsError> {
    getrandom::getrandom(buf).map_err(|_| KmsError::Random)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_printed_and_random() {
        let k = DataKey::from_bytes([7u8; 32]);
        assert_eq!(format!("{k:?}"), "DataKey(..)");
        assert_ne!(DataKey::random().unwrap(), DataKey::random().unwrap());
        assert!(DataKey::from_slice(&[1u8; 31], "test").is_err());
        assert_eq!(
            DataKey::from_slice(&[1u8; 32], "t").unwrap().expose(),
            &[1u8; 32]
        );
    }
}
