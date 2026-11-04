//! End-to-end tests driving the generated `widgets_router` with
//! `tower::ServiceExt::oneshot` — no live server, no database.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use seiva_controller_test::{widgets_router, WidgetService};
use seiva_http::{Principal, PROBLEM_JSON};

/// Reads a response body into a JSON value.
async fn json_body(body: Body) -> Value {
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// The guarded `create` route denies an anonymous caller with `403`, proving the
/// permission is enforced and read from the descriptor (`widgets.create`).
#[tokio::test]
async fn create_is_forbidden_without_permission() {
    let request = Request::builder()
        .method("POST")
        .uri("/v2/widgets")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"blocked"}"#))
        .unwrap();

    let response = widgets_router(WidgetService)
        .oneshot(request)
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response.headers()["content-type"],
        PROBLEM_JSON,
        "a denied request is served as application/problem+json"
    );
    let body = json_body(response.into_body()).await;
    assert_eq!(body["status"], 403);
    assert!(
        body["detail"].as_str().unwrap().contains("widgets.create"),
        "the detail names the permission read from the descriptor"
    );
}

/// A caller holding a different permission is still denied — the guard matches
/// the descriptor's permission exactly.
#[tokio::test]
async fn create_is_forbidden_with_wrong_permission() {
    let request = Request::builder()
        .method("POST")
        .uri("/v2/widgets")
        .header("content-type", "application/json")
        .extension(Principal::new("user", ["widgets.read"]))
        .body(Body::from(r#"{"name":"blocked"}"#))
        .unwrap();

    let response = widgets_router(WidgetService)
        .oneshot(request)
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// With the required permission and a valid body the guarded `create` route
/// succeeds with `201`, and the DTO comes back as JSON.
#[tokio::test]
async fn create_succeeds_with_permission() {
    let request = Request::builder()
        .method("POST")
        .uri("/v2/widgets")
        .header("content-type", "application/json")
        .extension(Principal::new("user", ["widgets.create"]))
        .body(Body::from(r#"{"name":"first"}"#))
        .unwrap();

    let response = widgets_router(WidgetService)
        .oneshot(request)
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = json_body(response.into_body()).await;
    assert_eq!(body["id"], 1);
    assert_eq!(body["name"], "first");
}

/// The path version comes from the descriptor (`v2`), not a hard-coded `v1`:
/// the `v1` path does not exist, so it is a `404`.
#[tokio::test]
async fn path_version_comes_from_descriptor() {
    // Wrong version: no such route.
    let wrong = Request::builder()
        .method("POST")
        .uri("/v1/widgets")
        .header("content-type", "application/json")
        .extension(Principal::new("user", ["widgets.create"]))
        .body(Body::from(r#"{"name":"x"}"#))
        .unwrap();
    let response = widgets_router(WidgetService).oneshot(wrong).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Right version: the route is mounted.
    let right = Request::builder()
        .method("POST")
        .uri("/v2/widgets")
        .header("content-type", "application/json")
        .extension(Principal::new("user", ["widgets.create"]))
        .body(Body::from(r#"{"name":"x"}"#))
        .unwrap();
    let response = widgets_router(WidgetService).oneshot(right).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

/// A body that parses but fails `Validate` (empty name) is a `422` RFC 9457
/// problem. The permission is supplied so the failure is the body, not the guard.
#[tokio::test]
async fn invalid_body_is_422_problem() {
    let request = Request::builder()
        .method("POST")
        .uri("/v2/widgets")
        .header("content-type", "application/json")
        .extension(Principal::new("user", ["widgets.create"]))
        .body(Body::from(r#"{"name":"   "}"#))
        .unwrap();

    let response = widgets_router(WidgetService)
        .oneshot(request)
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.headers()["content-type"], PROBLEM_JSON);
    let body = json_body(response.into_body()).await;
    assert_eq!(body["status"], 422);
    assert!(body["detail"].as_str().unwrap().contains("name"));
}

/// The `get` convention mounts `GET /v2/widgets/{id}` and extracts the id from
/// the path, calling the facade with it.
#[tokio::test]
async fn get_reads_id_from_path() {
    let request = Request::builder()
        .method("GET")
        .uri("/v2/widgets/42")
        .extension(Principal::new("user", ["widgets.read"]))
        .body(Body::empty())
        .unwrap();

    let response = widgets_router(WidgetService)
        .oneshot(request)
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response.into_body()).await;
    assert_eq!(body["id"], 42);
    assert_eq!(body["name"], "fetched");
}

/// The unguarded `list` route (no `#[requires]`) serves an anonymous caller a
/// `200`, proving an empty `requires` is treated as unguarded. `list` and
/// `create` share the `/v2/widgets` path with different verbs, proving the
/// per-route merge keeps both.
#[tokio::test]
async fn list_is_unguarded() {
    let request = Request::builder()
        .method("GET")
        .uri("/v2/widgets")
        .body(Body::empty())
        .unwrap();

    let response = widgets_router(WidgetService)
        .oneshot(request)
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response.into_body()).await;
    assert!(body.is_array());
    assert_eq!(body[0]["id"], 1);
}
