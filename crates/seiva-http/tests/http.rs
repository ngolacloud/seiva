//! End-to-end tests driving the router with `tower::ServiceExt::oneshot` — no
//! live server, no database, no broker, no example module.
//!
//! The framework router (`router`) is demo-free, so these tests cover it on its
//! own (health, `/docs`, `/openapi.json`) and exercise the guard + `Valid`
//! primitives through a small inline route built from the public API — the same
//! shape a generated controller produces, without depending on an example crate.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware::{from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tower::ServiceExt;

use seiva_http::{
    permission_guard, request_id, router, Health, Principal, Problem, RequiredPermission, Valid,
    Validate, ValidationError, PROBLEM_JSON, REQUEST_ID_HEADER,
};

/// Reads a response body into a JSON value.
async fn json_body(body: Body) -> Value {
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// --- An inline guarded + validated route, built from the public primitives ---

#[derive(Debug, Deserialize)]
struct DemoBody {
    title: String,
}

impl Validate for DemoBody {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.title.trim().is_empty() {
            return Err(ValidationError::new("title must not be empty"));
        }
        Ok(())
    }
}

async fn demo_create(Valid(body): Valid<DemoBody>) -> Result<Response, Problem> {
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": 1, "title": body.title })),
    )
        .into_response())
}

/// A route guarded by `demo.create` and validated by `Valid<DemoBody>`, wrapped
/// in the request-id middleware so a problem response carries a correlation id —
/// the exact composition a generated controller uses.
fn guarded_app() -> Router {
    Router::new()
        .route("/demo", post(demo_create))
        .layer(from_fn_with_state(
            RequiredPermission("demo.create"),
            permission_guard,
        ))
        .layer(from_fn(request_id))
}

#[tokio::test]
async fn guarded_route_happy_path_is_201_with_a_request_id() {
    let request = Request::builder()
        .method("POST")
        .uri("/demo")
        .header("content-type", "application/json")
        .extension(Principal::new("user-1", ["demo.create"]))
        .body(Body::from(r#"{"title":"first"}"#))
        .unwrap();

    let response = guarded_app().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert!(response.headers().contains_key(REQUEST_ID_HEADER));
    let body = json_body(response.into_body()).await;
    assert_eq!(body["title"], "first");
    assert_eq!(body["id"], 1);
}

#[tokio::test]
async fn guarded_route_without_permission_is_a_403_problem() {
    let request = Request::builder()
        .method("POST")
        .uri("/demo")
        .header("content-type", "application/json")
        .extension(Principal::new("user-2", ["demo.read"]))
        .body(Body::from(r#"{"title":"blocked"}"#))
        .unwrap();

    let response = guarded_app().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(response.headers()["content-type"], PROBLEM_JSON);
    let body = json_body(response.into_body()).await;
    assert_eq!(body["status"], 403);
    assert_eq!(body["title"], "Forbidden");
    assert!(body["detail"].as_str().unwrap().contains("demo.create"));
    assert!(body["request_id"].is_string());
}

#[tokio::test]
async fn guarded_route_anonymous_is_denied() {
    let request = Request::builder()
        .method("POST")
        .uri("/demo")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"title":"anon"}"#))
        .unwrap();
    let response = guarded_app().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn invalid_body_is_a_422_problem() {
    let request = Request::builder()
        .method("POST")
        .uri("/demo")
        .header("content-type", "application/json")
        .extension(Principal::new("user-1", ["demo.create"]))
        .body(Body::from(r#"{"title":"   "}"#))
        .unwrap();

    let response = guarded_app().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.headers()["content-type"], PROBLEM_JSON);
    let body = json_body(response.into_body()).await;
    assert_eq!(body["status"], 422);
    assert!(body["detail"].as_str().unwrap().contains("title"));
}

#[tokio::test]
async fn malformed_json_is_a_422() {
    let request = Request::builder()
        .method("POST")
        .uri("/demo")
        .header("content-type", "application/json")
        .extension(Principal::new("user-1", ["demo.create"]))
        .body(Body::from("not json"))
        .unwrap();
    let response = guarded_app().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

// --- The framework router, demo-free ----------------------------------------

/// `/health/live` is `200` in every state; `/health/ready` tracks the lifecycle.
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

    health.set_draining();
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

/// `/docs` serves the interactive reference over the service's own spec.
#[tokio::test]
async fn docs_page_is_served() {
    let response = router(Health::starting())
        .oneshot(Request::builder().uri("/docs").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
}

/// `/openapi.json` is a valid 3.1 document carrying the framework version. The
/// demo-free `seiva-http` links no facade, so `paths` is an empty object here;
/// the full runtime that mounts modules documents their operations.
#[tokio::test]
async fn openapi_document_is_served() {
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
    assert!(body["paths"].is_object());
}
