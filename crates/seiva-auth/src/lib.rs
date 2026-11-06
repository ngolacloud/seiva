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

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher as _, PasswordVerifier as _, SaltString,
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
    Argon2idHasher.hash(password)
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
    Argon2idHasher.verify(password, phc)
}

/// A password hashing backend — one algorithm that can hash a new password and
/// verify a stored PHC string it recognises. Implement this to add an algorithm
/// (bcrypt, scrypt, …) to a [`PasswordHashers`] registry **without editing this
/// crate** — the extension point modelled on Django's `PASSWORD_HASHERS`.
pub trait PasswordHasher: Send + Sync {
    /// The PHC algorithm identifier this backend produces and recognises, e.g.
    /// `"argon2id"`.
    fn id(&self) -> &'static str;

    /// Hashes `password`, returning a storable PHC string.
    ///
    /// # Errors
    /// Returns [`AuthError::Hash`] if the computation fails.
    fn hash(&self, password: &str) -> Result<String, AuthError>;

    /// Whether this backend recognises (and so can [`verify`](Self::verify)) `phc`.
    fn identifies(&self, phc: &str) -> bool;

    /// Verifies `password` against a PHC string this backend recognises.
    ///
    /// # Errors
    /// Returns [`AuthError::Phc`] if `phc` is malformed for this backend.
    fn verify(&self, password: &str, phc: &str) -> Result<bool, AuthError>;

    /// Whether a hash this backend produced should be re-made because its
    /// parameters drifted from the backend's current defaults. Defaults to `false`.
    fn needs_rehash(&self, _phc: &str) -> bool {
        false
    }
}

/// The default, preferred hasher: Argon2id with the current library defaults.
#[derive(Debug, Default, Clone, Copy)]
pub struct Argon2idHasher;

impl PasswordHasher for Argon2idHasher {
    fn id(&self) -> &'static str {
        "argon2id"
    }

    fn hash(&self, password: &str) -> Result<String, AuthError> {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|e| AuthError::Hash(e.to_string()))
    }

    fn identifies(&self, phc: &str) -> bool {
        PasswordHash::new(phc)
            .map(|p| p.algorithm.as_str() == "argon2id")
            .unwrap_or(false)
    }

    fn verify(&self, password: &str, phc: &str) -> Result<bool, AuthError> {
        let parsed = PasswordHash::new(phc).map_err(|e| AuthError::Phc(e.to_string()))?;
        match Argon2::default().verify_password(password.as_bytes(), &parsed) {
            Ok(()) => Ok(true),
            Err(argon2::password_hash::Error::Password) => Ok(false),
            Err(e) => Err(AuthError::Phc(e.to_string())),
        }
    }

    fn needs_rehash(&self, phc: &str) -> bool {
        let Ok(parsed) = PasswordHash::new(phc) else {
            return false;
        };
        let Ok(params) = argon2::Params::try_from(&parsed) else {
            return false;
        };
        let def = argon2::Params::default();
        params.m_cost() != def.m_cost()
            || params.t_cost() != def.t_cost()
            || params.p_cost() != def.p_cost()
    }
}

/// An ordered list of [`PasswordHasher`]s — Seiva's take on Django's
/// `PASSWORD_HASHERS`.
///
/// The first entry is **preferred**: it hashes new passwords. Verification tries
/// the first entry that [`identifies`](PasswordHasher::identifies) the stored PHC
/// string, so hashes made by any registered backend (including ones imported from
/// another system) verify. [`needs_rehash`](Self::needs_rehash) reports when a
/// stored hash should be upgraded on the next successful login.
pub struct PasswordHashers {
    hashers: Vec<Box<dyn PasswordHasher>>,
}

impl std::fmt::Debug for PasswordHashers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordHashers")
            .field(
                "ids",
                &self.hashers.iter().map(|h| h.id()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Default for PasswordHashers {
    /// Argon2id only — the same behaviour as the free [`hash_password`] /
    /// [`verify_password`] functions.
    fn default() -> Self {
        Self {
            hashers: vec![Box::new(Argon2idHasher)],
        }
    }
}

impl PasswordHashers {
    /// A registry whose preferred (new-password) hasher is `preferred`.
    pub fn new(preferred: impl PasswordHasher + 'static) -> Self {
        Self {
            hashers: vec![Box::new(preferred)],
        }
    }

    /// Adds a fallback hasher used only to *verify* existing hashes (new hashes
    /// always use the preferred one). Returns `self` so calls can be chained.
    #[must_use]
    pub fn with(mut self, fallback: impl PasswordHasher + 'static) -> Self {
        self.hashers.push(Box::new(fallback));
        self
    }

    fn preferred(&self) -> &dyn PasswordHasher {
        // Invariant: the registry is never empty (every constructor seeds one).
        self.hashers[0].as_ref()
    }

    /// Hashes a new password with the preferred hasher.
    ///
    /// # Errors
    /// Returns [`AuthError::Hash`] if the computation fails.
    pub fn hash(&self, password: &str) -> Result<String, AuthError> {
        self.preferred().hash(password)
    }

    /// Verifies `password` against `phc`, routing to the first registered hasher
    /// that recognises it.
    ///
    /// # Errors
    /// Returns [`AuthError::Phc`] if no registered hasher recognises `phc` (or the
    /// recognising hasher rejects it as malformed).
    pub fn verify(&self, password: &str, phc: &str) -> Result<bool, AuthError> {
        for hasher in &self.hashers {
            if hasher.identifies(phc) {
                return hasher.verify(password, phc);
            }
        }
        Err(AuthError::Phc(
            "no registered password hasher recognises this hash's algorithm".to_owned(),
        ))
    }

    /// Whether `phc` should be re-hashed on the next successful login: true when it
    /// was not made by the preferred hasher, or the preferred hasher reports its
    /// parameters drifted.
    pub fn needs_rehash(&self, phc: &str) -> bool {
        let preferred = self.preferred();
        if preferred.identifies(phc) {
            preferred.needs_rehash(phc)
        } else {
            true
        }
    }
}

/// The outcome of a successful [`authenticate_password`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Authenticated {
    /// The stored hash should be replaced with a fresh one (made by the preferred
    /// hasher) because it is legacy or its parameters drifted. The caller re-hashes
    /// the just-verified password and persists it.
    pub needs_rehash: bool,
}

/// Why [`authenticate_password`] refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginError {
    /// The key is locked out; the [`Duration`] is the time left on the lockout.
    LockedOut(Duration),
    /// The password did not match (the failure was recorded against the throttle).
    BadCredentials,
    /// The stored hash could not be used — a server/data error, not a wrong guess,
    /// so it is **not** counted against the throttle.
    Auth(AuthError),
}

impl std::fmt::Display for LoginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoginError::LockedOut(d) => write!(f, "account locked out for {}s", d.as_secs()),
            LoginError::BadCredentials => f.write_str("invalid credentials"),
            LoginError::Auth(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LoginError {}

/// The reference credential-verification flow that puts [`LoginThrottle`] into
/// service (OWASP API2) and drives upgrade-on-login.
///
/// It checks the lockout for `key`, verifies `password` against `stored_phc`
/// through `hashers`, records the success or failure against `throttle`, and on
/// success reports whether the stored hash
/// [`needs_rehash`](PasswordHashers::needs_rehash). An application's login endpoint
/// calls this, then (when `needs_rehash`) re-hashes with [`PasswordHashers::hash`]
/// and persists, and maps [`LoginError::LockedOut`] to `429` with a `Retry-After`.
///
/// Time is passed in (`now`) so the flow is deterministic and clock-free, matching
/// [`LoginThrottle`].
///
/// # Errors
/// Returns [`LoginError::LockedOut`] when `key` is locked, [`LoginError::BadCredentials`]
/// on a mismatch, or [`LoginError::Auth`] when `stored_phc` is unusable.
pub fn authenticate_password(
    throttle: &LoginThrottle,
    hashers: &PasswordHashers,
    key: &str,
    password: &str,
    stored_phc: &str,
    now: SystemTime,
) -> Result<Authenticated, LoginError> {
    if let Err(remaining) = throttle.check(key, now) {
        return Err(LoginError::LockedOut(remaining));
    }
    match hashers.verify(password, stored_phc) {
        Ok(true) => {
            throttle.record_success(key);
            Ok(Authenticated {
                needs_rehash: hashers.needs_rehash(stored_phc),
            })
        }
        Ok(false) => {
            throttle.record_failure(key, now);
            Err(LoginError::BadCredentials)
        }
        Err(e) => Err(LoginError::Auth(e)),
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

/// How aggressively [`LoginThrottle`] locks a key out after repeated failures.
///
/// A sane default (5 failures within 15 minutes → locked for 15 minutes) blunts
/// online password guessing (OWASP API2) without punishing a fat-fingered human.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThrottlePolicy {
    /// Failures within [`window`](Self::window) that trigger a lockout.
    pub max_failures: u32,
    /// The rolling window failures are counted over.
    pub window: Duration,
    /// How long a key stays locked once it trips the limit.
    pub lockout: Duration,
}

impl Default for ThrottlePolicy {
    fn default() -> Self {
        Self {
            max_failures: 5,
            window: Duration::from_secs(15 * 60),
            lockout: Duration::from_secs(15 * 60),
        }
    }
}

/// The per-key failure state the throttle tracks.
#[derive(Debug, Clone, Copy)]
struct Attempts {
    failures: u32,
    window_start: SystemTime,
    locked_until: Option<SystemTime>,
}

/// The ceiling on distinct keys the throttle tracks at once. Reaching it sweeps
/// keys that are neither locked nor inside their window (see
/// [`LoginThrottle::record_failure`]). It bounds the table so a flood of attempts
/// under many distinct usernames cannot exhaust memory — the lockout control must
/// not itself become a resource-consumption vector (OWASP API4). A key under
/// active attack keeps failures within its window, so it survives the sweep and
/// still locks out.
pub const MAX_TRACKED_KEYS: usize = 100_000;

/// An in-memory account-lockout throttle for authentication attempts (OWASP
/// API2: Broken Authentication).
///
/// It counts failed attempts per *key* — typically a username, or a
/// username+client-IP pair the caller composes — and locks the key out for
/// [`ThrottlePolicy::lockout`] once [`max_failures`](ThrottlePolicy::max_failures)
/// failures land within [`window`](ThrottlePolicy::window). A success clears the
/// key immediately.
///
/// Time is passed in by the caller (`now: SystemTime`) rather than read from a
/// global clock, so this crate stays dependency-free and the behaviour is tested
/// deterministically without sleeping. Production calls pass
/// [`SystemTime::now`]; a login middleware owns one `LoginThrottle` behind an
/// `Arc` and calls [`check`](Self::check) before verifying a password and
/// [`record_failure`](Self::record_failure)/[`record_success`](Self::record_success)
/// after.
///
/// ```
/// use seiva_auth::{LoginThrottle, ThrottlePolicy};
/// use std::time::{Duration, SystemTime};
///
/// let policy = ThrottlePolicy { max_failures: 3, window: Duration::from_secs(60), lockout: Duration::from_secs(300) };
/// let throttle = LoginThrottle::new(policy);
/// let now = SystemTime::UNIX_EPOCH;
///
/// // Three failures trip the lockout; the key is then refused.
/// for _ in 0..3 { throttle.record_failure("alice", now); }
/// assert!(throttle.check("alice", now).is_err());
///
/// // After the lockout elapses the key is allowed to try again.
/// assert!(throttle.check("alice", now + Duration::from_secs(301)).is_ok());
/// ```
#[derive(Debug)]
pub struct LoginThrottle {
    policy: ThrottlePolicy,
    keys: Mutex<HashMap<String, Attempts>>,
}

impl LoginThrottle {
    /// A throttle enforcing `policy`.
    #[must_use]
    pub fn new(policy: ThrottlePolicy) -> Self {
        Self {
            policy,
            keys: Mutex::new(HashMap::new()),
        }
    }

    /// Whether `key` may attempt an authentication now.
    ///
    /// # Errors
    ///
    /// Returns `Err(remaining)` — the time left on the lockout — when `key` is
    /// currently locked out.
    pub fn check(&self, key: &str, now: SystemTime) -> Result<(), Duration> {
        let keys = self.keys.lock().unwrap_or_else(|e| e.into_inner());
        match keys.get(key).and_then(|a| a.locked_until) {
            Some(until) => match until.duration_since(now) {
                Ok(remaining) if !remaining.is_zero() => Err(remaining),
                _ => Ok(()),
            },
            None => Ok(()),
        }
    }

    /// Records a failed attempt for `key`, returning `true` when this failure
    /// tripped the lockout. Failures outside the rolling window start a fresh
    /// count rather than accumulating forever.
    pub fn record_failure(&self, key: &str, now: SystemTime) -> bool {
        let mut keys = self.keys.lock().unwrap_or_else(|e| e.into_inner());

        // Bound the table before inserting a new key: a flood of distinct
        // usernames must not grow it without end.
        if keys.len() >= MAX_TRACKED_KEYS && !keys.contains_key(key) {
            // Keep only keys that still matter — locked out, or mid-window. A key
            // under active attack is mid-window, so it survives and still locks.
            keys.retain(|_, a| {
                a.locked_until.is_some_and(|until| until > now)
                    || now.duration_since(a.window_start).unwrap_or(Duration::ZERO)
                        < self.policy.window
            });
            // Still saturated with live state: do not track this new key (treat as
            // not-yet-locked). Targeting a specific account still works — that
            // account's key is live and tracked.
            if keys.len() >= MAX_TRACKED_KEYS && !keys.contains_key(key) {
                return false;
            }
        }

        let attempts = keys.entry(key.to_owned()).or_insert(Attempts {
            failures: 0,
            window_start: now,
            locked_until: None,
        });

        // A new window (the previous one elapsed) resets the count.
        let elapsed = now
            .duration_since(attempts.window_start)
            .unwrap_or(Duration::ZERO);
        if elapsed >= self.policy.window {
            attempts.window_start = now;
            attempts.failures = 0;
        }

        attempts.failures += 1;
        if attempts.failures >= self.policy.max_failures {
            attempts.locked_until = Some(now + self.policy.lockout);
            // Reset the counter so that, once the lockout expires, the key gets a
            // fresh window rather than being re-locked by one more failure.
            attempts.failures = 0;
            attempts.window_start = now;
            true
        } else {
            false
        }
    }

    /// Clears all failure state for `key` — called after a successful
    /// authentication so a user who eventually succeeds is not penalised.
    pub fn record_success(&self, key: &str) {
        let mut keys = self.keys.lock().unwrap_or_else(|e| e.into_inner());
        keys.remove(key);
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

    // --- Pluggable hashers + reference flow (ADR 0017) ----------------------

    /// A legacy test hasher: PHC `$plain$<password>`, always wants a rehash. It
    /// proves a non-Argon2id algorithm can be added by *extending* the registry.
    struct PlainHasher;
    impl PasswordHasher for PlainHasher {
        fn id(&self) -> &'static str {
            "plain"
        }
        fn hash(&self, password: &str) -> Result<String, AuthError> {
            Ok(format!("$plain${password}"))
        }
        fn identifies(&self, phc: &str) -> bool {
            phc.starts_with("$plain$")
        }
        fn verify(&self, password: &str, phc: &str) -> Result<bool, AuthError> {
            Ok(phc == format!("$plain${password}"))
        }
        fn needs_rehash(&self, _phc: &str) -> bool {
            true
        }
    }

    #[test]
    fn free_functions_still_roundtrip_through_the_default_registry() {
        let phc = hash_password("pw").unwrap();
        assert!(phc.starts_with("$argon2id$"));
        assert!(verify_password("pw", &phc).unwrap());
        assert!(!PasswordHashers::default().needs_rehash(&phc));
    }

    #[test]
    fn registry_verifies_a_legacy_algorithm_and_flags_it_for_rehash() {
        let hashers = PasswordHashers::default().with(PlainHasher);
        // New hashes use the preferred (argon2id) backend.
        let fresh = hashers.hash("pw").unwrap();
        assert!(fresh.starts_with("$argon2id$"));
        assert!(!hashers.needs_rehash(&fresh));
        // A legacy "$plain$" hash still verifies through the fallback…
        assert!(hashers.verify("pw", "$plain$pw").unwrap());
        assert!(!hashers.verify("wrong", "$plain$pw").unwrap());
        // …and is flagged to be upgraded on the next login.
        assert!(hashers.needs_rehash("$plain$pw"));
    }

    #[test]
    fn registry_rejects_an_unknown_algorithm() {
        let err = PasswordHashers::default()
            .verify("pw", "$bcrypt$whatever")
            .unwrap_err();
        assert!(matches!(err, AuthError::Phc(_)));
    }

    #[test]
    fn authenticate_succeeds_clears_throttle_and_reports_rehash() {
        let throttle = LoginThrottle::new(policy());
        let hashers = PasswordHashers::default().with(PlainHasher);
        let t0 = SystemTime::UNIX_EPOCH;
        // A fresh argon2id hash: authenticates, no rehash.
        let phc = hashers.hash("pw").unwrap();
        let ok = authenticate_password(&throttle, &hashers, "alice", "pw", &phc, t0).unwrap();
        assert!(!ok.needs_rehash);
        // A legacy hash: authenticates and asks for an upgrade-on-login.
        let up = authenticate_password(&throttle, &hashers, "bob", "pw", "$plain$pw", t0).unwrap();
        assert!(up.needs_rehash);
    }

    #[test]
    fn authenticate_locks_out_after_repeated_bad_passwords() {
        let throttle = LoginThrottle::new(policy()); // 3 failures → lockout
        let hashers = PasswordHashers::default();
        let phc = hashers.hash("right").unwrap();
        let t0 = SystemTime::UNIX_EPOCH;
        for _ in 0..3 {
            assert_eq!(
                authenticate_password(&throttle, &hashers, "mallory", "wrong", &phc, t0),
                Err(LoginError::BadCredentials)
            );
        }
        // Now locked out — even the correct password is refused until it elapses.
        match authenticate_password(&throttle, &hashers, "mallory", "right", &phc, t0) {
            Err(LoginError::LockedOut(_)) => {}
            other => panic!("expected LockedOut, got {other:?}"),
        }
    }

    #[test]
    fn authenticate_surfaces_a_bad_stored_hash_without_throttling() {
        let throttle = LoginThrottle::new(policy());
        let hashers = PasswordHashers::default();
        let t0 = SystemTime::UNIX_EPOCH;
        let err =
            authenticate_password(&throttle, &hashers, "dave", "pw", "not-a-phc", t0).unwrap_err();
        assert!(matches!(err, LoginError::Auth(_)));
        // A data error, not a guess: no failure recorded, the key is still allowed.
        assert!(throttle.check("dave", t0).is_ok());
    }

    // --- LoginThrottle (API2: account lockout) ------------------------------

    fn policy() -> ThrottlePolicy {
        ThrottlePolicy {
            max_failures: 3,
            window: Duration::from_secs(60),
            lockout: Duration::from_secs(300),
        }
    }

    #[test]
    fn locks_out_after_max_failures_and_releases_after_lockout() {
        let throttle = LoginThrottle::new(policy());
        let t0 = SystemTime::UNIX_EPOCH;

        assert!(throttle.check("alice", t0).is_ok());
        assert!(!throttle.record_failure("alice", t0));
        assert!(!throttle.record_failure("alice", t0));
        // The third failure trips the lockout.
        assert!(throttle.record_failure("alice", t0));
        assert!(throttle.check("alice", t0).is_err());

        // Still locked just before the lockout elapses...
        assert!(throttle
            .check("alice", t0 + Duration::from_secs(299))
            .is_err());
        // ...and allowed again once it has.
        assert!(throttle
            .check("alice", t0 + Duration::from_secs(301))
            .is_ok());
    }

    #[test]
    fn success_clears_failures() {
        let throttle = LoginThrottle::new(policy());
        let t0 = SystemTime::UNIX_EPOCH;
        throttle.record_failure("bob", t0);
        throttle.record_failure("bob", t0);
        throttle.record_success("bob");
        // After a success the count is gone: two fresh failures do not lock out.
        assert!(!throttle.record_failure("bob", t0));
        assert!(!throttle.record_failure("bob", t0));
        assert!(throttle.check("bob", t0).is_ok());
    }

    #[test]
    fn failures_outside_the_window_do_not_accumulate() {
        let throttle = LoginThrottle::new(policy());
        let t0 = SystemTime::UNIX_EPOCH;
        throttle.record_failure("carol", t0);
        throttle.record_failure("carol", t0);
        // A third failure well after the window starts a fresh count, so it does
        // not trip the lockout.
        assert!(!throttle.record_failure("carol", t0 + Duration::from_secs(120)));
        assert!(throttle
            .check("carol", t0 + Duration::from_secs(120))
            .is_ok());
    }

    #[test]
    fn keys_are_independent() {
        let throttle = LoginThrottle::new(policy());
        let t0 = SystemTime::UNIX_EPOCH;
        for _ in 0..3 {
            throttle.record_failure("attacker", t0);
        }
        assert!(throttle.check("attacker", t0).is_err());
        // A different key is unaffected.
        assert!(throttle.check("victim", t0).is_ok());
    }

    #[test]
    fn the_key_table_is_bounded_and_a_targeted_account_still_locks() {
        let throttle = LoginThrottle::new(policy());
        let t0 = SystemTime::UNIX_EPOCH;

        // A specific account under attack: it must still lock out despite the flood.
        for _ in 0..3 {
            throttle.record_failure("target", t0);
        }
        assert!(throttle.check("target", t0).is_err());

        // Flood far more distinct usernames than the cap, all within the window.
        for i in 0..(MAX_TRACKED_KEYS as u64 + 5_000) {
            let _ = throttle.record_failure(&format!("junk-{i}"), t0);
        }

        let tracked = throttle.keys.lock().unwrap().len();
        assert!(
            tracked <= MAX_TRACKED_KEYS,
            "key table must stay bounded, got {tracked}"
        );
        // The real target's lockout survived the sweep.
        assert!(throttle.check("target", t0).is_err());
    }
}
