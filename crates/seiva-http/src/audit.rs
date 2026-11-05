//! Persistent audit of administrative (and other sensitive) mutations (ADR 0013).
//!
//! [`audit`] is a middleware that records every **mutating** request
//! (`POST`/`PUT`/`PATCH`/`DELETE`) to an [`AuditSink`]: the acting principal, the
//! tenant, the operation (`METHOD path`), the result status and the correlation
//! id — including denied attempts (a `403`), which are exactly what an operator
//! needs to see. It records *metadata*, never the request body, so no secret can
//! leak through it; a handler that wants field-level detail builds its own
//! [`AuditEntry`] (redacting with [`AuditEntry::redacted`]) and records it inside
//! its transaction.
//!
//! Mount it inside the authn layer so the `Principal` is populated, and give it a
//! sink (the DORM-backed `DormAuditSink`, or an in-memory one in tests). A failed
//! audit write never fails the request — the mutation already happened — but it
//! is logged at `warn`, never swallowed.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use seiva_identity::Principal;
use seiva_ports::{AuditEntry, AuditSink};

use crate::request_id::RequestIdExt;

/// Whether a method mutates state and so is worth auditing.
fn is_mutation(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

/// Middleware that records every mutating request to `sink`. Mount with
/// [`axum::middleware::from_fn_with_state`]`(sink, audit)`, inside the authn layer.
pub async fn audit(
    State(sink): State<Arc<dyn AuditSink>>,
    request: Request,
    next: Next,
) -> Response {
    if !is_mutation(request.method()) {
        return next.run(request).await;
    }

    // Capture what we need before the request is consumed by the inner service.
    let principal = request
        .extensions()
        .get::<Principal>()
        .cloned()
        .unwrap_or_else(Principal::anonymous);
    let operation = format!("{} {}", request.method(), request.uri().path());
    let request_id = request.extensions().request_id();

    let response = next.run(request).await;

    let entry = AuditEntry {
        actor: principal.subject,
        tenant: principal.tenant,
        operation,
        object: None,
        outcome: response.status().as_u16().to_string(),
        request_id,
        details: Vec::new(),
    }
    .redacted();

    if let Err(error) = sink.record(&entry).await {
        // Best-effort: the mutation already produced its response, so a failed
        // audit write cannot un-happen it — but it must be surfaced, not swallowed.
        tracing::warn!(target: "seiva::audit", %error, "audit record failed");
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use axum::middleware::from_fn_with_state;
    use axum::routing::{get, post};
    use axum::Router;
    use seiva_testing::RecordingAuditSink;
    use tower::ServiceExt;

    fn app(sink: RecordingAuditSink) -> Router {
        let state: Arc<dyn AuditSink> = Arc::new(sink);
        Router::new()
            .route("/things", post(|| async { StatusCode::CREATED }))
            .route("/things", get(|| async { "list" }))
            .layer(from_fn_with_state(state, audit))
    }

    #[tokio::test]
    async fn records_a_mutation_with_actor_and_outcome() {
        let sink = RecordingAuditSink::new();
        let app = app(sink.clone());
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/things")
            .extension(Principal::in_tenant("alice", "acme", ["things.create"]))
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(request).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);

        let entries = sink.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].actor, "alice");
        assert_eq!(entries[0].tenant.as_deref(), Some("acme"));
        assert_eq!(entries[0].operation, "POST /things");
        assert_eq!(entries[0].outcome, "201");
    }

    #[tokio::test]
    async fn a_denied_mutation_is_still_audited() {
        // A handler that denies (403) — the attempt must still be recorded.
        let sink = RecordingAuditSink::new();
        let state: Arc<dyn AuditSink> = Arc::new(sink.clone());
        let app = Router::new()
            .route("/things", post(|| async { StatusCode::FORBIDDEN }))
            .layer(from_fn_with_state(state, audit));
        let res = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/things")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let entries = sink.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].outcome, "403");
        assert_eq!(entries[0].actor, "", "anonymous actor is empty");
    }

    #[tokio::test]
    async fn a_read_is_not_audited() {
        let sink = RecordingAuditSink::new();
        let app = app(sink.clone());
        let res = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/things")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(sink.entries().is_empty(), "a GET must not be audited");
    }
}
