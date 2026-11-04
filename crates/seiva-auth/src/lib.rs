//! Credential primitives for Seiva — the framework's password and permission
//! building blocks, moved out of DORM's CLI accounts (the brief's Parte
//! DORM-A). This crate is **DORM-free** and **data-layer-free**: it holds no
//! database handle and never reaches the ORM.
//!
//! What it offers:
//!
//! - [`hash_password`] / [`verify_password`]: Argon2id hashing producing and
//!   checking a [PHC string](https://github.com/P-H-C/phc-string-format)
//!   (`$argon2id$v=19$m=...,t=...,p=...$<salt>$<hash>`). A fresh random salt is
//!   drawn per hash, so two hashes of the same password differ.
//! - [`prompt_password`]: a thin no-echo console prompt (the terminal stops
//!   echoing keystrokes while the secret is typed).
//! - [`Permission`] and [`PermissionSet`]: a minimal named-permission value
//!   type (for example `notes.create`) and a set with membership checks.
//!
//! ```
//! # use seiva_auth::{hash_password, verify_password};
//! let phc = hash_password("correct horse battery staple").unwrap();
//! assert!(phc.starts_with("$argon2id$"));
//! assert!(verify_password("correct horse battery staple", &phc).unwrap());
//! assert!(!verify_password("wrong", &phc).unwrap());
//! ```

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;

/// What a credential operation can refuse.
///
/// Kept deliberately small (a single owned message per variant) so that a
/// `Result<_, AuthError>` returned by value stays well under clippy's
/// `result_large_err` threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// The Argon2id hashing operation failed.
    Hash(String),
    /// A stored PHC string could not be parsed (malformed or unsupported).
    Phc(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Hash(m) => write!(f, "password hashing failed: {m}"),
            AuthError::Phc(m) => write!(f, "invalid PHC hash string: {m}"),
        }
    }
}

impl std::error::Error for AuthError {}

/// Hashes a password with Argon2id and returns a PHC string.
///
/// A random salt is generated from the operating system's CSPRNG
/// ([`OsRng`]), and the Argon2id default parameters are used. The returned
/// value is a self-describing PHC string beginning with `$argon2id$`, suitable
/// for storage; the parameters and salt it embeds are all [`verify_password`]
/// needs to check a candidate later.
///
/// # Errors
///
/// Returns [`AuthError::Hash`] if the underlying Argon2id computation fails.
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| AuthError::Hash(e.to_string()))
}

/// Verifies a password against a PHC string produced by [`hash_password`].
///
/// Returns `Ok(true)` on a match and `Ok(false)` on a mismatch. The Argon2id
/// parameters and salt are read from the PHC string itself, so no configuration
/// is needed here.
///
/// # Errors
///
/// Returns [`AuthError::Phc`] if `phc` is not a parseable PHC hash string.
pub fn verify_password(password: &str, phc: &str) -> Result<bool, AuthError> {
    let parsed = PasswordHash::new(phc).map_err(|e| AuthError::Phc(e.to_string()))?;
    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(AuthError::Phc(e.to_string())),
    }
}

/// Prompts on the terminal for a password without echoing the keystrokes.
///
/// A thin wrapper over [`rpassword`]; the prompt is written and the typed line
/// is returned with its trailing newline stripped. Interactive by nature, so it
/// is not unit-tested.
///
/// # Errors
///
/// Returns any [`std::io::Error`] raised while reading from the terminal.
pub fn prompt_password(prompt: &str) -> std::io::Result<String> {
    rpassword::prompt_password(prompt)
}

/// A single named permission, for example `notes.create`.
///
/// A thin, comparable wrapper over an owned permission name. The framework
/// attaches no meaning to the string beyond equality; dotted segments are a
/// convention, not a parsed structure.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Permission(String);

impl Permission {
    /// Creates a permission from any string-like name.
    pub fn new(name: impl Into<String>) -> Self {
        Permission(name.into())
    }

    /// Borrows the permission's name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for Permission {
    fn from(name: &str) -> Self {
        Permission::new(name)
    }
}

impl From<String> for Permission {
    fn from(name: String) -> Self {
        Permission(name)
    }
}

/// An unordered set of [`Permission`]s with membership checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionSet {
    items: std::collections::HashSet<Permission>,
}

impl PermissionSet {
    /// Creates an empty set.
    pub fn new() -> Self {
        PermissionSet::default()
    }

    /// Inserts a permission, returning `self` so inserts can be chained.
    #[must_use]
    pub fn with(mut self, permission: impl Into<Permission>) -> Self {
        self.items.insert(permission.into());
        self
    }

    /// Inserts a permission in place.
    pub fn insert(&mut self, permission: impl Into<Permission>) {
        self.items.insert(permission.into());
    }

    /// Reports whether the set contains the given permission.
    pub fn contains(&self, permission: &Permission) -> bool {
        self.items.contains(permission)
    }

    /// Returns the number of permissions in the set.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Reports whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl FromIterator<Permission> for PermissionSet {
    fn from_iter<I: IntoIterator<Item = Permission>>(iter: I) -> Self {
        PermissionSet {
            items: iter.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_roundtrips() {
        let phc = hash_password("s3cr3t-passphrase").unwrap();
        assert!(verify_password("s3cr3t-passphrase", &phc).unwrap());
    }

    #[test]
    fn wrong_password_verifies_false() {
        let phc = hash_password("s3cr3t-passphrase").unwrap();
        assert!(!verify_password("not-the-password", &phc).unwrap());
    }

    #[test]
    fn hash_is_a_phc_argon2id_string() {
        let phc = hash_password("whatever").unwrap();
        assert!(
            phc.starts_with("$argon2id$"),
            "expected a PHC argon2id string, got: {phc}"
        );
    }

    #[test]
    fn same_password_hashes_differ_due_to_random_salt() {
        let a = hash_password("same-input").unwrap();
        let b = hash_password("same-input").unwrap();
        assert_ne!(a, b, "random salt must make two hashes differ");
    }

    #[test]
    fn verify_rejects_a_malformed_phc_string() {
        let err = verify_password("secret", "not-a-phc-string").unwrap_err();
        assert!(matches!(err, AuthError::Phc(_)));
    }

    #[test]
    fn permission_set_membership() {
        let set = PermissionSet::new()
            .with("notes.create")
            .with(Permission::new("notes.read"));
        assert!(set.contains(&Permission::from("notes.create")));
        assert!(!set.contains(&Permission::new("notes.delete")));
        assert_eq!(set.len(), 2);
        assert!(!set.is_empty());
    }
}
