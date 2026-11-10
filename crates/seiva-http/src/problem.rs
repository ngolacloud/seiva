//! RFC 9457 `application/problem+json` error responses.
//!
//! [`Problem`] — the transport-neutral error DATA — lives in `seiva-contract`
//! (ADR 0003), so a module's `facade` can map its error to a `Problem` without
//! depending on this HTTP crate. This module re-exports it and provides
//! [`ProblemResponse`], the axum `IntoResponse` wrapper that renders a `Problem`
//! as `application/problem+json`. Every extractor rejection and middleware
//! response in this crate wraps the `Problem` in a `ProblemResponse`, and the
//! `#[seiva::controller]` macro does the same at the HTTP edge.

use axum::body::Body;
use axum::http::header::CONTENT_TYPE;
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};

// The error data and the fail-closed expose flag are transport-neutral, so they
// live in `seiva-contract`; re-exported here so `seiva_http::Problem` and
// `seiva_http::problem::{set_expose_error_details, expose_error_details}` are
// unchanged for every existing caller.
pub use seiva_contract::{expose_error_details, set_expose_error_details, Problem};

/// The media type mandated by RFC 9457 for a problem document.
pub const PROBLEM_JSON: &str = "application/problem+json";

/// The axum `IntoResponse` wrapper around a transport-neutral [`Problem`].
///
/// `Problem` itself is transport-free (it has no `IntoResponse`), so the HTTP edge
/// wraps it in this to render it: `application/problem+json` with the problem's
/// status, mirroring the `request_id` into an `x-request-id` header. Extractors use
/// it as their `Rejection`, middleware return it, and the generated controller maps
/// a facade's `Problem` error through it.
#[derive(Debug, Clone)]
pub struct ProblemResponse(pub Problem);

impl From<Problem> for ProblemResponse {
    fn from(problem: Problem) -> Self {
        Self(problem)
    }
}

impl IntoResponse for ProblemResponse {
    fn into_response(self) -> Response {
        let problem = self.0;
        let body = serde_json::to_vec(&problem.to_json()).unwrap_or_else(|_| b"{}".to_vec());
        let mut response = Response::new(Body::from(body));
        *response.status_mut() = problem.status;
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static(PROBLEM_JSON));
        if let Some(request_id) = &problem.request_id {
            if let Ok(value) = HeaderValue::from_str(request_id) {
                response.headers_mut().insert("x-request-id", value);
            }
        }
        response
    }
}
