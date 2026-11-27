//! `seiva-kms` — envelope encryption with a key-encryption key held by a
//! guardian (ADR 0049).
//!
//! ```text
//!   secret ──seal(aad)──▶ sv1:<key-id>:…        (KeyRing: data keys in memory)
//!                              ▲
//!          data keys ── wrapped by the KEK ──▶ WrappedRing (stored by RingStore)
//!                              ▲
//!                   KeyWrapper: LocalKek (file) · VaultTransit (Vault transit)
//! ```
//!
//! - **Envelope.** The key-encryption key (KEK) never leaves its guardian: in
//!   Vault it is the transit key; with [`LocalKek`] it lives only in the
//!   process's memory, read from a mounted file. Data keys are 32 random bytes,
//!   stored only wrapped by the KEK ([`WrappedRing`]).
//! - **Bound to context.** [`KeyRing::seal`] takes [`Aad`] —
//!   `<org>|<purpose>|<record>` — and the blob opens only with the same bytes.
//!   Copied to another row, account or organisation, it does not open.
//! - **Rotation without downtime.** The current data key seals, earlier keys
//!   only open; [`RingKeeper::rotate`] adds a key and asks every [`Reseal`] to
//!   re-seal, [`RingKeeper::retire`] drops the earlier keys once nothing needs
//!   them. A KEK rotated in the guardian is picked up by re-wrapping the ring
//!   ([`KeyWrapper::rewrap`]) at the next [`RingKeeper::load`].
//! - **Fails closed.** A ring that does not parse, a key that is not unwrapped,
//!   a blob that does not authenticate: each is an error, never a fallback.
//!   [`RingKeeper::boot`] retries an unreachable guardian until a deadline, then
//!   fails.
//! - **dxstorage compatible.** [`KeyRing::open_kms1`] opens dxstorage's
//!   `kms1:` blobs and [`WrappedRing::from_dxstorage_json`] reads its stored
//!   ring, so it can move to this crate without re-sealing everything at once.
//!
//! The caller decides where rings live ([`RingStore`], compare-and-swap) and how
//! its secrets are re-sealed ([`Reseal`]). [`MemoryRingStore`] is the in-memory
//! store for tests.
//!
//! Features: `local` (default) — [`LocalKek`]; `vault` — `VaultTransit` over
//! reqwest + rustls (token, AppRole or Kubernetes auth, an extra CA, a start-up
//! deadline). AWS KMS, Google Cloud KMS and Azure Key Vault come later behind the
//! same [`KeyWrapper`] port.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod error;
mod format;
mod keeper;
mod key;
#[cfg(feature = "local")]
mod local;
mod ring;
mod store;
#[cfg(feature = "vault")]
mod vault;
mod wrapper;

pub use error::KmsError;
pub use format::{valid_key_id, Aad, SealedFormat, KMS1_PREFIX, SV1_PREFIX};
pub use keeper::{retry_until, Retirement, RingKeeper, Rotation};
pub use key::DataKey;
#[cfg(feature = "local")]
pub use local::LocalKek;
pub use ring::{KeyRing, WrappedKey, WrappedRing, RING_FORMAT};
pub use store::{MemoryRingStore, Reseal, RingStore, StoredRing};
#[cfg(feature = "vault")]
pub use vault::{wrap_version, VaultAuth, VaultConfig, VaultTransit, K8S_TOKEN_FILE};
pub use wrapper::{GeneratedKey, KeyWrapper};

/// The plaintext an open returns: wiped from memory when dropped.
pub use zeroize::Zeroizing;
