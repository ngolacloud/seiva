//! Facade layer of the `accounts` module: the versioned public contract and
//! DTOs.
//!
//! This is what other modules see. The `#[seiva::facade]` macro keeps the trait
//! as the source of truth and generates the `AccountsFacade` descriptor (ADR
//! 0001). The DTOs crossing this boundary carry **no secrets**: neither the
//! plaintext password nor the stored hash ever appears in a response type.

#![forbid(unsafe_code)]
#![allow(async_fn_in_trait)]

use delonix_seiva::facade;

/// Command DTO: the input to [`Accounts::create_user`].
///
/// Carries the plaintext `password` inbound only. It is hashed in the
/// application layer (Argon2id) and never stored or echoed back; nothing
/// returned from this facade exposes it.
#[derive(Debug, Clone)]
pub struct CreateUser {
    /// The desired, unique username.
    pub username: String,
    /// The plaintext password. Hashed by the application layer; never persisted
    /// or returned.
    pub password: String,
}

/// Response DTO: what `create_user`/`get_user` return. Never a domain type, and
/// deliberately **without** any password or hash field — a secret cannot leak
/// through this type because it has nowhere to sit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserDto {
    /// The account's stable identifier.
    pub id: u128,
    /// The account's username.
    pub username: String,
}

/// The `accounts` module's public contract.
#[facade(version = "v1")]
pub trait Accounts {
    /// Creates a user from a [`CreateUser`] command, returning the new
    /// account's public [`UserDto`]. Rejects a duplicate username.
    #[requires("accounts.create")]
    async fn create_user(&self, cmd: CreateUser) -> Result<UserDto, String>;

    /// Looks a user up by username, returning its public [`UserDto`].
    #[requires("accounts.read")]
    async fn get_user(&self, username: String) -> Result<UserDto, String>;
}
