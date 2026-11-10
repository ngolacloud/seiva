//! Facade layer of the `accounts` module: the versioned public contract and the
//! wire DTOs.
//!
//! This is what other modules and the HTTP/gRPC transports see. The
//! `#[seiva::facade]` macro keeps the trait as the source of truth and generates
//! the `AccountsFacade` descriptor (ADR 0001). The DTOs crossing this boundary
//! carry **no secrets**: neither the plaintext password nor the stored hash ever
//! appears in a response type.
//!
//! The generated `#[seiva::controller]` (wired in `accounts-interface`) uses the
//! facade's OWN DTOs as the wire types, so — mirroring the scaffold-generated
//! module and `examples/notes` — the write DTOs derive `serde` and implement
//! [`seiva_contract::Validate`], the response DTO derives [`serde::Serialize`], and
//! the error maps to an RFC 9457 [`Problem`] here. Hence this layer depends on
//! `delonix-seiva`, `serde` and `seiva-http`.

#![forbid(unsafe_code)]
#![allow(async_fn_in_trait)]

use delonix_seiva::{facade, WireSchema};
use seiva_contract::{Problem, Validate, ValidationError};
use seiva_http::Ctx;
use serde::{Deserialize, Serialize};

/// Command DTO: the input to [`Accounts::create_user`] and the JSON body of the
/// generated `create_user` route, validated by the controller's `Valid<T>`
/// extractor.
///
/// Carries the plaintext `password` inbound only. It is hashed in the
/// application layer (Argon2id) and never stored or echoed back; nothing
/// returned from this facade exposes it.
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct CreateUser {
    /// The desired, unique username.
    pub username: String,
    /// The plaintext password. Hashed by the application layer; never persisted
    /// or returned.
    pub password: String,
}

impl Validate for CreateUser {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.username.trim().is_empty() {
            return Err(ValidationError::new("username must not be empty"));
        }
        if self.password.is_empty() {
            return Err(ValidationError::new("password must not be empty"));
        }
        Ok(())
    }
}

/// Query DTO: the input to [`Accounts::get_user`] and the JSON body of the
/// generated `get_user` route.
///
/// The read takes a DTO (not a bare `String`) so the controller's `Valid<T>`
/// extractor can deserialise and validate it: `seiva_contract::Validate` cannot be
/// implemented for `String` (the orphan rule), so a bare-`String` parameter
/// could not be mounted as an HTTP route. See the module docs on the generated
/// routes for the resulting path shape.
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct GetUser {
    /// The username to look up.
    pub username: String,
}

impl Validate for GetUser {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.username.trim().is_empty() {
            return Err(ValidationError::new("username must not be empty"));
        }
        Ok(())
    }
}

/// Response DTO: what `create_user`/`get_user` return. Never a domain type, and
/// deliberately **without** any password or hash field — a secret cannot leak
/// through this type because it has nowhere to sit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, WireSchema)]
pub struct UserDto {
    /// The account's stable identifier.
    pub id: u128,
    /// The account's username.
    pub username: String,
}

/// The facade's error type. The controller maps it to an RFC 9457 problem via
/// this `Into<Problem>`, so the module owns how its failures surface. A plain
/// message wrapper (like the scaffold's `RecordError`), kept distinct from the
/// application layer's internal `AccountsError` so no internal detail leaks
/// beyond the message the application chose to expose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserError(pub String);

impl std::fmt::Display for UserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UserError {}

impl From<UserError> for Problem {
    fn from(error: UserError) -> Self {
        Problem::unprocessable_entity(error.0)
    }
}

/// The `accounts` module's public contract.
///
/// Each method takes the framework request context `&Ctx` (the authenticated
/// caller: tenant + permissions). The `#[seiva::controller]` macro **skips it from
/// the wire** (ADR 0006) and injects it from the request, so it never appears in a
/// DTO; the interface derives the tenant [`Scope`](seiva_http) from it **server-side**
/// — the tenant is never read from the request body (OWASP API1). The application
/// stays HTTP-free: it receives a `Scope`, not a `Ctx`.
#[facade(version = "v1")]
pub trait Accounts {
    /// Creates a user from a [`CreateUser`] command, under the caller's tenant,
    /// returning the new account's public [`UserDto`]. Rejects a duplicate
    /// username **within that tenant**.
    #[requires("accounts.create")]
    async fn create_user(&self, ctx: &Ctx, cmd: CreateUser) -> Result<UserDto, UserError>;

    /// Looks a user up by username (carried in a [`GetUser`] query) **within the
    /// caller's tenant**, returning its public [`UserDto`].
    #[requires("accounts.read")]
    async fn get_user(&self, ctx: &Ctx, query: GetUser) -> Result<UserDto, UserError>;
}
