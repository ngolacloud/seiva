//! RFC 9457 `application/problem+json` error responses.
//!
//! [`Problem`] is the single error shape every handler and extractor returns. It
//! carries the standard members (`type`, `title`, `status`, `detail`,
//! `instance`) plus a `request_id` extension so a client and an operator can
//! correlate a failure to a single request. Its [`IntoResponse`] serializes to
//! `application/problem+json` with the right status code, and a framework
//! [`BoxError`] maps into a `500` problem.

use std::sync::atomic::{AtomicBool, Ordering};

use axum::body::Body;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use seiva_ports::BoxError;

/// The media type mandated by RFC 9457 for a problem document.
pub const PROBLEM_JSON: &str = "application/problem+json";

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

/// An RFC 9457 problem detail: a machine-readable description of an HTTP error.
///
/// Construct one with a status helper ([`Problem::forbidden`],
/// [`Problem::unprocessable_entity`], [`Problem::internal`], …) and refine it
/// with the builder methods. The response body is the JSON object RFC 9457
/// defines, with `request_id` added as an extension member.
///
/// The string members are stored as `Box<str>` rather than `String`: a problem
/// is built once and never mutated after construction, so the extra `String`
/// capacity word buys nothing, and the smaller footprint keeps the whole value
/// comfortably under clippy's `result_large_err` threshold. That is why no crate
/// returning `Result<_, Problem>` needs to silence that lint.
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

    /// A `401 Unauthorized` problem (the caller presented a bearer credential
    /// that did not authenticate). The caller is expected to add the
    /// `WWW-Authenticate` response header separately.
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

    /// A `422 Unprocessable Entity` problem (the body failed to parse or to
    /// validate).
    pub fn unprocessable_entity(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, "Unprocessable Entity").with_detail(detail)
    }

    /// A `429 Too Many Requests` problem (the caller exceeded a rate limit). The
    /// caller is expected to add a `Retry-After` response header separately.
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

    /// The problem document as a JSON value, omitting absent optional members.
    fn to_json(&self) -> serde_json::Value {
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

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let body = serde_json::to_vec(&self.to_json()).unwrap_or_else(|_| b"{}".to_vec());
        let mut response = Response::new(Body::from(body));
        *response.status_mut() = self.status;
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static(PROBLEM_JSON));
        if let Some(request_id) = &self.request_id {
            if let Ok(value) = HeaderValue::from_str(request_id) {
                response.headers_mut().insert("x-request-id", value);
            }
        }
        response
    }
}

/// Maps the framework's opaque port error into a problem response.
///
/// A port error that is **already a client-facing [`Problem`]** is recovered
/// verbatim, status and all: a pipeline behavior or a handler that returned a
/// [`Problem`] boxed as the port's `BoxError` (the `403` from
/// [`crate::bus::Authorize`], the `409` from [`crate::bus::Idempotent`]) keeps its
/// status instead of being flattened to `500`. This is what makes those behaviors
/// *status-bearing*: the error type is the single source of truth for the status,
/// so the HTTP edge does not have to re-derive it. Only an explicitly-constructed
/// [`Problem`] matches this path — an opaque adapter/database error does not
/// downcast and still maps to `500`.
///
/// For every other error the internal `Display` is **not** sent to the client by
/// default: it is logged server-side (correlate via the response's `request_id`)
/// and the client receives a generic detail, so an adapter/database message never
/// leaks (OWASP API8). Setting `SEIVA_EXPOSE_ERROR_DETAILS` (development only — the
/// production config guard rejects it) flips [`set_expose_error_details`] and the
/// real text is surfaced to speed up local debugging.
impl From<BoxError> for Problem {
    fn from(error: BoxError) -> Self {
        // Recover a Problem that travelled inside the box (a behavior's 403/409),
        // preserving its status; fall through for any other, opaque error.
        match error.downcast::<Problem>() {
            Ok(problem) => *problem,
            Err(error) => {
                if expose_error_details() {
                    Problem::internal(error.to_string())
                } else {
                    // Logged, not returned: the operator sees the cause, the client does not.
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
