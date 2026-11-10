//! Transport-neutral wire-contract types for the Delonix Seiva framework.
//!
//! A module's `facade` is its versioned public contract. The write DTOs it
//! publishes carry a [`Validate`] check — a structural, synchronous validation
//! that is a *contract* concern, not a *transport* one. These types live here,
//! with **no axum, no http, no framework**, so the `facade` layer can validate
//! without depending on a transport crate (ADR 0003). `seiva-http` re-exports
//! them (`seiva_http::{Validate, ValidationError}`), so existing code keeps
//! compiling and the HTTP `Valid<T>` extractor runs the same check.
//!
//! ```
//! use seiva_contract::{Validate, ValidationError};
//!
//! struct CreateRecord { title: String }
//! impl Validate for CreateRecord {
//!     fn validate(&self) -> Result<(), ValidationError> {
//!         if self.title.trim().is_empty() {
//!             return Err(ValidationError::new("title must not be empty"));
//!         }
//!         Ok(())
//!     }
//! }
//! assert!(CreateRecord { title: "  ".into() }.validate().is_err());
//! ```

#![forbid(unsafe_code)]

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

// --- RFC 9457 Problem -------------------------------------------------------
//
// `Problem` is the error a use case maps to at the module boundary. It is pure
// DATA (an `http::StatusCode` + the RFC 9457 members) with NO axum / response
// impl, so the `facade` layer declares its error without naming a transport
// crate. The HTTP transport (`seiva-http`) wraps it in a `ProblemResponse` that
// implements `axum::IntoResponse`; another transport maps it its own way.

use std::sync::atomic::{AtomicBool, Ordering};

use http::StatusCode;
use seiva_ports::BoxError;

/// Whether a `500` may carry the internal error's text to the client. **Off by
/// default (fail-closed):** an internal error that reaches [`From<BoxError>`] is
/// logged server-side and the client sees only a generic detail, so a database or
/// adapter message (SQL, column names, paths) never leaks (OWASP API8/API5). The
/// runtime flips this from `SEIVA_EXPOSE_ERROR_DETAILS` at boot
/// ([`set_expose_error_details`]); the production config guard rejects enabling it
/// outside development.
static EXPOSE_ERROR_DETAILS: AtomicBool = AtomicBool::new(false);

/// Sets whether internal error detail is exposed to clients. Called once by the
/// runtime at boot from the validated `SEIVA_EXPOSE_ERROR_DETAILS` config value. A
/// development convenience; production must leave it off (fail-closed default).
pub fn set_expose_error_details(expose: bool) {
    EXPOSE_ERROR_DETAILS.store(expose, Ordering::Relaxed);
}

/// Whether internal error detail is currently exposed to clients.
#[must_use]
pub fn expose_error_details() -> bool {
    EXPOSE_ERROR_DETAILS.load(Ordering::Relaxed)
}

/// An RFC 9457 problem detail: a machine-readable description of an error.
///
/// Construct one with a status helper ([`Problem::forbidden`],
/// [`Problem::unprocessable_entity`], [`Problem::internal`], …) and refine it with
/// the builder methods. The transport renders it (`seiva-http` as
/// `application/problem+json`); here it is transport-neutral data.
///
/// The string members are stored as `Box<str>` rather than `String`: a problem is
/// built once and never mutated after construction, so the extra `String` capacity
/// word buys nothing, and the smaller footprint keeps the whole value comfortably
/// under clippy's `result_large_err` threshold. That is why no crate returning
/// `Result<_, Problem>` needs to silence that lint.
#[derive(Debug, Clone)]
pub struct Problem {
    /// A URI reference identifying the problem type. Defaults to `about:blank`,
    /// which RFC 9457 treats as "no extra semantics beyond the status code".
    pub type_uri: Box<str>,
    /// A short, human-readable summary of the problem type.
    pub title: Box<str>,
    /// The HTTP status code.
    pub status: StatusCode,
    /// A human-readable explanation specific to this occurrence.
    pub detail: Option<Box<str>>,
    /// A URI reference identifying this specific occurrence (e.g. the path).
    pub instance: Option<Box<str>>,
    /// The id of the request that produced the problem, for correlation.
    pub request_id: Option<Box<str>>,
}

impl Problem {
    /// A problem with the given status and title, `type` defaulting to
    /// `about:blank` and no detail/instance/request id yet.
    pub fn new(status: StatusCode, title: impl Into<String>) -> Self {
        Self {
            type_uri: "about:blank".into(),
            title: title.into().into_boxed_str(),
            status,
            detail: None,
            instance: None,
            request_id: None,
        }
    }

    /// A `401 Unauthorized` problem (the caller presented a bearer credential that
    /// did not authenticate). The caller adds the `WWW-Authenticate` header.
    pub fn unauthorized(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "Unauthorized").with_detail(detail)
    }

    /// A `403 Forbidden` problem (the permission guard denied the caller).
    pub fn forbidden(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "Forbidden").with_detail(detail)
    }

    /// A `404 Not Found` problem.
    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "Not Found").with_detail(detail)
    }

    /// A `422 Unprocessable Entity` problem (the body failed to parse or validate).
    pub fn unprocessable_entity(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, "Unprocessable Entity").with_detail(detail)
    }

    /// A `429 Too Many Requests` problem (the caller exceeded a rate limit). The
    /// caller adds a `Retry-After` header separately.
    pub fn too_many_requests(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "Too Many Requests").with_detail(detail)
    }

    /// A `500 Internal Server Error` problem.
    pub fn internal(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").with_detail(detail)
    }

    /// Sets the `type` URI.
    #[must_use]
    pub fn with_type(mut self, type_uri: impl Into<String>) -> Self {
        self.type_uri = type_uri.into().into_boxed_str();
        self
    }

    /// Sets the human-readable `detail`.
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into().into_boxed_str());
        self
    }

    /// Sets the `instance` URI reference.
    #[must_use]
    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into().into_boxed_str());
        self
    }

    /// Sets the correlation `request_id`.
    #[must_use]
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into().into_boxed_str());
        self
    }

    /// Sets the `request_id` when one is present, leaving it unset otherwise.
    #[must_use]
    pub fn with_request_id_opt(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id.map(String::into_boxed_str);
        self
    }

    /// The problem document as a JSON value, omitting absent optional members. The
    /// transport serialises this (e.g. as `application/problem+json`).
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        map.insert("type".to_owned(), self.type_uri.as_ref().into());
        map.insert("title".to_owned(), self.title.as_ref().into());
        map.insert("status".to_owned(), self.status.as_u16().into());
        if let Some(detail) = &self.detail {
            map.insert("detail".to_owned(), detail.as_ref().into());
        }
        if let Some(instance) = &self.instance {
            map.insert("instance".to_owned(), instance.as_ref().into());
        }
        if let Some(request_id) = &self.request_id {
            map.insert("request_id".to_owned(), request_id.as_ref().into());
        }
        serde_json::Value::Object(map)
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.status.as_u16(), self.title)?;
        if let Some(detail) = &self.detail {
            write!(f, ": {detail}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Problem {}

/// Maps the framework's opaque port error into a problem.
///
/// A port error that is **already a client-facing [`Problem`]** is recovered
/// verbatim, status and all: a pipeline behavior or handler that returned a
/// [`Problem`] boxed as the port's `BoxError` (a `403` from an authorize behavior,
/// a `409` from an idempotency behavior) keeps its status instead of being
/// flattened to `500`. Only an explicitly-constructed [`Problem`] matches this
/// path — an opaque adapter/database error does not downcast and maps to `500`.
///
/// For every other error the internal `Display` is **not** sent to the client by
/// default: it is logged server-side (correlate via the response's `request_id`)
/// and the client receives a generic detail, so an adapter/database message never
/// leaks (OWASP API8). Setting `SEIVA_EXPOSE_ERROR_DETAILS` (development only)
/// flips [`set_expose_error_details`] and the real text is surfaced.
impl From<BoxError> for Problem {
    fn from(error: BoxError) -> Self {
        match error.downcast::<Problem>() {
            Ok(problem) => *problem,
            Err(error) => {
                if expose_error_details() {
                    Problem::internal(error.to_string())
                } else {
                    tracing::error!(error = %error, "internal error mapped to 500");
                    Problem::internal("an internal error occurred")
                }
            }
        }
    }
}

// Regression guard for the `result_large_err` win: should a future field push
// `Problem` back over clippy's 128-byte threshold, the blanket allows return and
// the `check` CI job fails on the integration test again. Fail the build here
// instead, at the type, with a message pointing at the cause.
const _: () = assert!(
    std::mem::size_of::<Problem>() <= 112,
    "Problem must stay small enough that Result<_, Problem> does not trip \
     clippy::result_large_err; box or restructure a field before growing it",
);

#[cfg(test)]
mod tests {
    use super::*;

    struct Name(String);

    impl Validate for Name {
        fn validate(&self) -> Result<(), ValidationError> {
            if self.0.trim().is_empty() {
                return Err(ValidationError::new("name must not be empty"));
            }
            Ok(())
        }
    }

    #[test]
    fn validate_rejects_and_accepts() {
        assert!(Name("   ".into()).validate().is_err());
        assert!(Name("ada".into()).validate().is_ok());
    }

    #[test]
    fn validation_error_carries_the_message_as_display() {
        let error = ValidationError::new("bad");
        assert_eq!(error.message, "bad");
        assert_eq!(error.to_string(), "bad");
    }
}
