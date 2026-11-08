//! End-to-end proof of the live choreography `orders.placed → inventory.reserved`
//! (ADR 0021 Fase 3), fully OFFLINE — no database, no broker.
//!
//! It drives the REAL composed shop service ([`shop_api::build_app`]): it seeds
//! stock and places an order over HTTP (exactly as a client would, through the demo
//! `shop-token` bearer), then runs the in-process outbox driver
//! ([`drain_in_process`]) and asserts the subscriber reserved the ordered stock in
//! the SAME inventory state the HTTP controller serves. A second drain over a
//! redelivered event proves the inbox makes the reservation happen exactly once.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use seiva_events::drain_in_process;
use shop_api::build_app;
use tower::ServiceExt;

/// The demo bearer the composition root wires; it holds every shop permission.
const TOKEN: &str = "shop-token";

/// A `product_id` to exercise the whole flow with.
const PRODUCT: &str = "42";

/// POST `uri` with `body` as an authenticated `shop-token` request and return the
/// status. The router is cloned per call so each `oneshot` consumes its own copy.
async fn post(app: &axum::Router, uri: &str, body: &str) -> StatusCode {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::from(body.to_owned()))
        .unwrap();
    app.clone().oneshot(request).await.unwrap().status()
}

/// The units currently on hand for `PRODUCT`, read straight from the SHARED stock
/// repository the controller and the subscriber both hold.
fn available(app: &shop_api::BuiltApp) -> u64 {
    let pid: u128 = PRODUCT.parse().unwrap();
    app.inventory_repo
        .items()
        .into_iter()
        .find(|s| s.product_id == pid)
        .map(|s| s.available)
        .unwrap_or(0)
}

#[tokio::test]
async fn placing_an_order_reserves_stock_through_the_in_process_dispatch() {
    let app = build_app();

    // (1) Seed stock over HTTP: 10 units of the product.
    assert_eq!(
        post(
            &app.router,
            "/v1/inventories/receive",
            &format!(r#"{{"product_id":"{PRODUCT}","qty":10}}"#),
        )
        .await,
        StatusCode::OK,
    );
    assert_eq!(available(&app), 10, "stock seeded over HTTP");

    // (2) Place an order for 3 units of that product over HTTP. The `orders` module
    // publishes `orders.placed` into the shared outbox inside its transaction.
    assert_eq!(
        post(
            &app.router,
            "/v1/orders/place",
            &format!(r#"{{"customer":"Ada","lines":[{{"product_id":"{PRODUCT}","qty":3}}]}}"#),
        )
        .await,
        StatusCode::OK,
    );
    assert_eq!(
        app.outbox.pending(),
        1,
        "placing the order wrote exactly one event to the outbox"
    );
    assert_eq!(
        available(&app),
        10,
        "no stock is reserved until the event is dispatched"
    );

    // (3) Drain the outbox: the dispatcher routes `orders.placed` to the inventory
    // subscriber, which reserves the ordered units through the real use case.
    let dispatched = drain_in_process(&*app.outbox, &app.registry, app.inbox.as_ref())
        .await
        .unwrap();
    assert_eq!(dispatched, 1, "one event was dispatched");

    // (4) The stock dropped by EXACTLY the ordered quantity, in the same state the
    // HTTP controller serves — the choreography closed the loop.
    assert_eq!(
        available(&app),
        7,
        "the subscriber reserved 3 of the 10 seeded units"
    );
    assert_eq!(
        app.outbox.pending(),
        0,
        "the cursor advanced past the event"
    );

    // (Extra) Model an at-least-once redelivery: rewind the cursor and drain again.
    // The event is re-read and dispatched, but the inbox absorbs it, so NO second
    // reservation happens — exactly-one effect per (subscriber, event).
    app.outbox.redeliver(0);
    let redispatched = drain_in_process(&*app.outbox, &app.registry, app.inbox.as_ref())
        .await
        .unwrap();
    assert_eq!(
        redispatched, 1,
        "the redelivered event was dispatched again"
    );
    assert_eq!(
        available(&app),
        7,
        "the inbox made the reservation idempotent — stock is unchanged"
    );
}
