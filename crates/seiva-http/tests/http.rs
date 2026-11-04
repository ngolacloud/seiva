//! End-to-end tests driving the router with `tower::ServiceExt::oneshot` — no
//! live server, no database, no broker.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use seiva_http::{router, Health, Principal, PROBLEM_JSON, REQUEST_ID_HEADER};

/// Reads a response body into a JSON value.
async fn json_body(body: Body) -> Value {
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// A `POST /v1/notes` carrying a principal that holds `notes.create` and a valid
/// body is created (`201`) and the DTO comes back as JSON.
#[tokio::test]
async fn create_note_happy_path() {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/notes")
        .header("content-type", "application/json")
        .extension(Principal::new("user-1", ["notes.create"]))
        .body(Body::from(r#"{"title":"first note"}"#))
        .unwrap();

    let response = router(Health::starting()).oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    // The request_id middleware stamped a correlation id on the response.
    assert!(response.headers().contains_key(REQUEST_ID_HEADER));

    let body = json_body(response.into_body()).await;
    assert_eq!(body["title"], "first note");
    assert_eq!(body["id"], 1);
}

/// Without the `notes.create` permission the descriptor-driven guard denies the
/// request with a `403` RFC 9457 problem carrying the required permission and a
/// request id.
#[tokio::test]
async fn create_note_forbidden_without_permission() {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/notes")
        .header("content-type", "application/json")
        // A principal that holds a different permission.
        .extension(Principal::new("user-2", ["notes.read"]))
        .body(Body::from(r#"{"title":"blocked"}"#))
        .unwrap();

    let response = router(Health::starting()).oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response.headers()["content-type"],
        PROBLEM_JSON,
        "a problem is served as application/problem+json"
    );

    let body = json_body(response.into_body()).await;
    assert_eq!(body["status"], 403);
    assert_eq!(body["title"], "Forbidden");
    assert!(
        body["detail"].as_str().unwrap().contains("notes.create"),
        "the detail names the permission read from the descriptor"
    );
    assert!(body["request_id"].is_string());
}

/// Entirely missing principal (anonymous) is denied, proving the default is
/// "hold nothing", not "admit".
#[tokio::test]
async fn create_note_forbidden_when_anonymous() {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/notes")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"title":"anon"}"#))
        .unwrap();

    let response = router(Health::starting()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// A body that parses but fails `Validate` (empty title) is a `422` problem.
#[tokio::test]
async fn create_note_invalid_body_is_422() {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/notes")
        .header("content-type", "application/json")
        .extension(Principal::new("user-1", ["notes.create"]))
        .body(Body::from(r#"{"title":"   "}"#))
        .unwrap();

    let response = router(Health::starting()).oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.headers()["content-type"], PROBLEM_JSON);

    let body = json_body(response.into_body()).await;
    assert_eq!(body["status"], 422);
    assert!(body["detail"].as_str().unwrap().contains("title"));
}

/// Malformed JSON is also a `422` problem.
#[tokio::test]
async fn create_note_malformed_json_is_422() {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/notes")
        .header("content-type", "application/json")
        .extension(Principal::new("user-1", ["notes.create"]))
        .body(Body::from("not json"))
        .unwrap();

    let response = router(Health::starting()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// `/health/live` is `200` in every state; `/health/ready` is `503` until ready,
/// then `200`, then `503` again while draining.
#[tokio::test]
async fn health_endpoints_track_lifecycle() {
    let health = Health::starting();

    let live = router(health.clone())
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(live.status(), StatusCode::OK);

    // Starting: not ready.
    let ready = router(health.clone())
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);

    // Ready: serving.
    health.set_ready();
    let ready = router(health.clone())
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ready.status(), StatusCode::OK);
    let body = json_body(ready.into_body()).await;
    assert_eq!(body["state"], "ready");

    // Draining: alive but out of rotation.
    health.set_draining();
    let live = router(health.clone())
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(live.status(), StatusCode::OK);
    let ready = router(health.clone())
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
}

/// The OpenAPI stub is served and advertises the framework version.
#[tokio::test]
async fn openapi_stub_is_served() {
    let response = router(Health::starting())
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response.into_body()).await;
    assert_eq!(body["openapi"], "3.1.0");
    assert!(body["info"]["version"].is_string());
}
