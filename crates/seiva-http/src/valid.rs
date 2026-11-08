//! A typed, validated JSON body extractor.
//!
//! [`Valid<T>`] is the `Valid<Json<T>>`-style extractor: it reads the request
//! body as JSON into `T`, then runs `T`'s [`Validate`] check. A parse failure or
//! a validation failure both become a `422`
//! [`Problem`]; otherwise the handler receives a value
//! already known to be well-formed and valid.

use axum::body::Bytes;
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

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let request_id = request.extensions().request_id();
        let instance = request.uri().path().to_owned();
        // Use axum's `Bytes` extractor rather than `into_body()` + `to_bytes(_,
        // usize::MAX)`: `Bytes` honours the router's `DefaultBodyLimit`, so an
        // oversized body is rejected before it is buffered into memory (OWASP
        // API4 — resource exhaustion). Reading with `usize::MAX` silently
        // bypassed that limit for every generated write route.
        let bytes = Bytes::from_request(request, state).await.map_err(|error| {
            // Keep the rejection's own status (a limit breach is `413 Payload Too
            // Large`, a read failure `400`); the body text is axum's fixed,
            // non-sensitive message. The title tracks the status.
            let status = error.status();
            Problem::new(status, status.canonical_reason().unwrap_or("Bad Request"))
                .with_detail(error.body_text())
                .with_instance(instance.clone())
                .with_request_id_opt(request_id.clone())
        })?;

        // Reject unknown fields (OWASP API6 — mass assignment): a body carrying
        // keys the DTO does not declare is refused instead of silently dropped,
        // so a client cannot smuggle server-controlled fields (`role`, `tenant`,
        // `is_admin`) past a DTO that happens to omit `#[serde(deny_unknown_fields)]`.
        // `serde_ignored` reports each key the DTO ignored; the first one fails
        // the request. This holds for every DTO without touching the wire shape
        // of valid requests.
        let mut unknown_field: Option<String> = None;
        let deserializer = &mut serde_json::Deserializer::from_slice(&bytes);
        let value: T = serde_ignored::deserialize(deserializer, |path| {
            if unknown_field.is_none() {
                unknown_field = Some(path.to_string());
            }
        })
        .map_err(|error| {
            Problem::unprocessable_entity(format!("invalid JSON body: {error}"))
                .with_instance(instance.clone())
                .with_request_id_opt(request_id.clone())
        })?;

        if let Some(field) = unknown_field {
            return Err(Problem::unprocessable_entity(format!(
                "unknown field `{field}` is not accepted"
            ))
            .with_instance(instance.clone())
            .with_request_id_opt(request_id.clone()));
        }

        value.validate().map_err(|error| {
            Problem::unprocessable_entity(error.to_string())
                .with_instance(instance)
                .with_request_id_opt(request_id)
        })?;

        Ok(Valid(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::StatusCode;

    #[derive(Debug, serde::Deserialize)]
    struct Dto {
        name: String,
    }

    impl Validate for Dto {
        fn validate(&self) -> Result<(), ValidationError> {
            if self.name.is_empty() {
                return Err(ValidationError::new("name must not be empty"));
            }
            Ok(())
        }
    }

    async fn extract(json: &str) -> Result<Valid<Dto>, Problem> {
        let request = Request::builder()
            .uri("/v1/things")
            .body(Body::from(json.to_owned()))
            .unwrap();
        Valid::<Dto>::from_request(request, &()).await
    }

    #[tokio::test]
    async fn accepts_a_body_with_exactly_the_declared_fields() {
        let valid = extract(r#"{"name":"widget"}"#)
            .await
            .expect("should accept");
        assert_eq!(valid.0.name, "widget");
    }

    #[tokio::test]
    async fn rejects_an_unknown_field_as_422_mass_assignment_guard() {
        // A client smuggling a server-controlled field the DTO does not declare is
        // refused, not silently dropped (OWASP API6).
        let err = extract(r#"{"name":"widget","is_admin":true}"#)
            .await
            .expect_err("unknown field must be rejected");
        assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(err
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("is_admin"));
    }

    #[tokio::test]
    async fn still_runs_the_validate_check() {
        let err = extract(r#"{"name":""}"#)
            .await
            .expect_err("empty name invalid");
        assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}
