//! End-to-end proof of the live choreography `orders.placed → inventory.reserved`
//! (ADR 0021 Fase 3 + Fase 2), fully OFFLINE — no database, no broker.
//!
//! It drives the REAL composed shop service ([`shop_api::build_app`]) with the
//! in-process dispatch worker spawned exactly as `main` spawns it
//! ([`BuiltApp::spawn_dispatch_worker`]). It seeds stock and places an order over HTTP
//! (through the demo `shop-token` bearer), then — **without calling the driver by
//! hand** — waits for the worker to drain the outbox and asserts the subscriber
//! reserved the ordered stock in the SAME inventory state the HTTP controller serves.
//! This is the Fase 2 proof that the choreography progresses on its own with the app
//! running.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
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

/// Poll `available(app)` until it equals `want`, or panic after ~2 s — so the test
/// observes the BACKGROUND worker's progress without ever driving the drain itself.
async fn wait_until_available(app: &shop_api::BuiltApp, want: u64) {
    for _ in 0..200 {
        if available(app) == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "timed out waiting for stock to reach {want}; still {} — the worker did not \
         progress the choreography",
        available(app)
    );
}

#[tokio::test]
async fn placing_an_order_reserves_stock_through_the_spawned_dispatch_worker() {
    let app = build_app();

    // Spawn the in-process dispatch worker exactly as `main` does: from here on the
    // choreography progresses on its own, with no `drain_in_process` call in the test.
    let _worker = app.spawn_dispatch_worker();

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
    // publishes `orders.placed` into the shared outbox inside its transaction; the
    // background worker will pick it up.
    assert_eq!(
        post(
            &app.router,
            "/v1/orders/place",
            &format!(r#"{{"customer":"Ada","lines":[{{"product_id":"{PRODUCT}","qty":3}}]}}"#),
        )
        .await,
        StatusCode::OK,
    );

    // (3) WITHOUT draining by hand, the worker dispatches `orders.placed` to the
    // inventory subscriber, which reserves the ordered units through the real use
    // case — the stock drops by exactly the ordered quantity, in the same state the
    // HTTP controller serves. This is the choreography closing the loop on its own.
    wait_until_available(&app, 7).await;
    assert_eq!(
        app.outbox.pending(),
        0,
        "the worker advanced the cursor past the dispatched event"
    );

    // (4) Placing a SECOND order draws the stock down again — proving the worker
    // keeps draining, not just the first event.
    assert_eq!(
        post(
            &app.router,
            "/v1/orders/place",
            &format!(r#"{{"customer":"Grace","lines":[{{"product_id":"{PRODUCT}","qty":2}}]}}"#),
        )
        .await,
        StatusCode::OK,
    );
    wait_until_available(&app, 5).await;
}
