//! Per-request correlation id.
//!
//! [`request_id`] is an axum middleware that assigns every request a
//! [`RequestId`] (honouring an inbound `x-request-id` when present), stores it in
//! the request extensions for extractors and the permission guard to read, and
//! echoes it on the response. [`Problem`](crate::problem::Problem) carries it in
//! the body so a client can quote one id to an operator.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;

/// The header carrying the correlation id, in and out.
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// A request's correlation id, stored in the request extensions by
/// [`request_id`] and readable by extractors via [`RequestIdExt`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestId(pub String);

impl RequestId {
    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Mints a fresh id without pulling in a UUID dependency: the wall-clock time in
/// nanoseconds, salted by a monotonic process-wide counter so two requests in the
/// same nanosecond still differ.
fn mint() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:032x}{seq:016x}")
}

/// Convenience to read the [`RequestId`] out of a set of request extensions.
pub trait RequestIdExt {
    /// The correlation id, if the [`request_id`] middleware ran.
    fn request_id(&self) -> Option<String>;
}

impl RequestIdExt for axum::http::Extensions {
    fn request_id(&self) -> Option<String> {
        self.get::<RequestId>().map(|id| id.0.clone())
    }
}

/// Middleware that assigns a [`RequestId`], stores it in the extensions and
/// echoes it on the response's `x-request-id` header. Reuses an inbound
/// `x-request-id` when the client sent a syntactically valid one.
pub async fn request_id(mut request: Request, next: Next) -> Response {
    let id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(mint);

    request.extensions_mut().insert(RequestId(id.clone()));
    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    response
}
