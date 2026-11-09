//! End-to-end proof that `POST /v1/orders/place` dedupes through the command bus
//! (ADR 0020 Fase 3 + ADR 0011), fully OFFLINE — no database, no broker.
//!
//! It drives the REAL composed shop service ([`shop_api::build_app`], which injects
//! an idempotency store into the `orders` controller) and proves the header →
//! `Ctx` → bus wiring end to end: two `POST`s carrying the SAME `Idempotency-Key`
//! create exactly ONE order and write exactly ONE `orders.placed` event to the
//! outbox, while the retry replays the first response. A request without the header
//! still succeeds (idempotency is opt-in), and the choreography it drives is left
//! untouched — see `choreography_e2e.rs`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use shop_api::build_app;
use tower::ServiceExt;

/// The demo bearer the composition root wires; it holds every shop permission.
const TOKEN: &str = "shop-token";

/// POST `uri` with `body`, optionally carrying an `Idempotency-Key`, as an
/// authenticated `shop-token` request. Returns the status and the JSON response body.
async fn post(
    app: &axum::Router,
    uri: &str,
    body: &str,
    idempotency_key: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TOKEN}"));
    if let Some(key) = idempotency_key {
        builder = builder.header("idempotency-key", key);
    }
    let request = builder.body(Body::from(body.to_owned())).unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, json)
}

/// How many orders the service currently holds (read over HTTP through `list`).
async fn order_count(app: &axum::Router) -> usize {
    let request = Request::builder()
        .method("GET")
        .uri("/v1/orders")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let listed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    listed.as_array().unwrap().len()
}

#[tokio::test]
async fn two_posts_with_the_same_idempotency_key_place_exactly_one_order() {
    let app = build_app();
    let body = r#"{"customer":"Ada","lines":[{"product_id":"42","qty":3}]}"#;

    // First request with a key: a fresh order is placed.
    let (s1, first) = post(&app.router, "/v1/orders/place", body, Some("order-key-1")).await;
    assert_eq!(s1, StatusCode::OK);

    // Retry with the SAME key + SAME body: the bus replays the stored response.
    let (s2, second) = post(&app.router, "/v1/orders/place", body, Some("order-key-1")).await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(first, second, "the retry replays the first response");

    // Exactly one order and exactly one outbox event — the write ran once.
    assert_eq!(
        order_count(&app.router).await,
        1,
        "the bus dedup produced exactly one order"
    );
    assert_eq!(
        app.outbox.pending(),
        1,
        "exactly one `orders.placed` event was published"
    );
}

#[tokio::test]
async fn the_same_key_with_different_content_is_a_409_conflict() {
    let app = build_app();

    let (s1, _) = post(
        &app.router,
        "/v1/orders/place",
        r#"{"customer":"Ada","lines":[{"product_id":"42","qty":3}]}"#,
        Some("order-key-2"),
    )
    .await;
    assert_eq!(s1, StatusCode::OK);

    // Same key, different (valid) body → fingerprint mismatch → 409, handler not run.
    let (s2, _) = post(
        &app.router,
        "/v1/orders/place",
        r#"{"customer":"Ada","lines":[{"product_id":"42","qty":9}]}"#,
        Some("order-key-2"),
    )
    .await;
    assert_eq!(s2, StatusCode::CONFLICT);

    assert_eq!(
        order_count(&app.router).await,
        1,
        "the mismatched retry never placed a second order"
    );
}

#[tokio::test]
async fn a_request_without_a_key_still_places_the_order() {
    let app = build_app();
    let (status, _) = post(
        &app.router,
        "/v1/orders/place",
        r#"{"customer":"Ada","lines":[{"product_id":"42","qty":1}]}"#,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "idempotency is opt-in; a plain request still runs"
    );
    assert_eq!(order_count(&app.router).await, 1);
}
