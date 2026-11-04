//! RFC 9457 `application/problem+json` error responses.
//!
//! [`Problem`] is the single error shape every handler and extractor returns. It
//! carries the standard members (`type`, `title`, `status`, `detail`,
//! `instance`) plus a `request_id` extension so a client and an operator can
//! correlate a failure to a single request. Its [`IntoResponse`] serializes to
//! `application/problem+json` with the right status code, and a framework
//! [`BoxError`] maps into a `500` problem.

use axum::body::Body;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use seiva_ports::BoxError;

/// The media type mandated by RFC 9457 for a problem document.
pub const PROBLEM_JSON: &str = "application/problem+json";

/// An RFC 9457 problem detail: a machine-readable description of an HTTP error.
///
/// Construct one with a status helper ([`Problem::forbidden`],
/// [`Problem::unprocessable_entity`], [`Problem::internal`], …) and refine it
/// with the builder methods. The response body is the JSON object RFC 9457
/// defines, with `request_id` added as an extension member.
#[derive(Debug, Clone)]
pub struct Problem {
    /// A URI reference identifying the problem type. Defaults to `about:blank`,
    /// which RFC 9457 treats as "no extra semantics beyond the status code".
    pub type_uri: String,
    /// A short, human-readable summary of the problem type.
    pub title: String,
    /// The HTTP status code.
    pub status: StatusCode,
    /// A human-readable explanation specific to this occurrence.
    pub detail: Option<String>,
    /// A URI reference identifying this specific occurrence (e.g. the path).
    pub instance: Option<String>,
    /// The id of the request that produced the problem, for correlation.
    pub request_id: Option<String>,
}

impl Problem {
    /// A problem with the given status and title, `type` defaulting to
    /// `about:blank` and no detail/instance/request id yet.
    pub fn new(status: StatusCode, title: impl Into<String>) -> Self {
        Self {
            type_uri: "about:blank".to_owned(),
            title: title.into(),
            status,
            detail: None,
            instance: None,
            request_id: None,
        }
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
        self.type_uri = type_uri.into();
        self
    }

    /// Sets the human-readable `detail`.
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Sets the `instance` URI reference.
    #[must_use]
    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    /// Sets the correlation `request_id`.
    #[must_use]
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    /// Sets the `request_id` when one is present, leaving it unset otherwise.
    #[must_use]
    pub fn with_request_id_opt(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
        self
    }

    /// The problem document as a JSON value, omitting absent optional members.
    fn to_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        map.insert("type".to_owned(), self.type_uri.clone().into());
        map.insert("title".to_owned(), self.title.clone().into());
        map.insert("status".to_owned(), self.status.as_u16().into());
        if let Some(detail) = &self.detail {
            map.insert("detail".to_owned(), detail.clone().into());
        }
        if let Some(instance) = &self.instance {
            map.insert("instance".to_owned(), instance.clone().into());
        }
        if let Some(request_id) = &self.request_id {
            map.insert("request_id".to_owned(), request_id.clone().into());
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

/// Maps the framework's opaque port error into a `500` problem. The error's
/// `Display` becomes the `detail`; adapters are expected to keep sensitive
/// specifics out of it (first cut — a later phase will add a classification seam
/// so some port errors map to `409`/`422` instead of a blanket `500`).
impl From<BoxError> for Problem {
    fn from(error: BoxError) -> Self {
        Problem::internal(error.to_string())
    }
}
