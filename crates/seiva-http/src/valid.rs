//! A typed, validated JSON body extractor.
//!
//! [`Valid<T>`] is the `Valid<Json<T>>`-style extractor: it reads the request
//! body as JSON into `T`, then runs `T`'s [`Validate`] check. A parse failure or
//! a validation failure both become a `422`
//! [`Problem`]; otherwise the handler receives a value
//! already known to be well-formed and valid.

use axum::extract::{FromRequest, Request};
use serde::de::DeserializeOwned;

use crate::problem::Problem;
use crate::request_id::RequestIdExt;

/// A domain-shaped validation check on a deserialized body. Keep it cheap and
/// synchronous: structural and range checks, not I/O. Business rules that need
/// ports belong in the application layer, not here.
pub trait Validate {
    /// Returns `Ok(())` when the value is acceptable, or a human-readable reason
    /// why it is not.
    fn validate(&self) -> Result<(), ValidationError>;
}

/// Why a body was rejected as invalid. A single message in this first cut; a
/// later phase can carry a per-field map.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// A human-readable reason, surfaced as the problem's `detail`.
    pub message: String,
}

impl ValidationError {
    /// A validation error with the given message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ValidationError {}

/// A request body that deserialized into `T` and passed `T`'s [`Validate`]
/// check. Destructure it as `Valid(value)` in a handler.
#[derive(Debug, Clone, Copy)]
pub struct Valid<T>(pub T);

impl<T, S> FromRequest<S> for Valid<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(request: Request, _state: &S) -> Result<Self, Self::Rejection> {
        let request_id = request.extensions().request_id();
        let instance = request.uri().path().to_owned();
        let bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
            .await
            .map_err(|error| {
                Problem::unprocessable_entity(format!("could not read request body: {error}"))
                    .with_instance(instance.clone())
                    .with_request_id_opt(request_id.clone())
            })?;

        let value: T = serde_json::from_slice(&bytes).map_err(|error| {
            Problem::unprocessable_entity(format!("invalid JSON body: {error}"))
                .with_instance(instance.clone())
                .with_request_id_opt(request_id.clone())
        })?;

        value.validate().map_err(|error| {
            Problem::unprocessable_entity(error.to_string())
                .with_instance(instance)
                .with_request_id_opt(request_id)
        })?;

        Ok(Valid(value))
    }
}
