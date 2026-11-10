//! An end-to-end demonstration of the idempotency building blocks wired into a
//! handler the way the application layer uses them (ADR 0011): the transport
//! validates the `Idempotency-Key` and computes the canonical fingerprint, and
//! the handler drives `IdempotencyStore` so the effect runs at most once.
//!
//! The durable guarantee is proved against a real database elsewhere
//! (`seiva_data_dorm::live_idempotency_one_effect_under_concurrency`); here the
//! in-memory double lets the HTTP wiring and the Fresh→Replay→Mismatch behaviour
//! be exercised with no database.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use tower::ServiceExt;

use seiva_http::{
    fingerprint, in_progress_problem, mismatch_problem, scoped_key, OptionalIdempotencyKey,
    Principal, Problem, ProblemResponse,
};
use seiva_ports::{IdempotencyOutcome, IdempotencyStore};
use seiva_testing::InMemoryIdempotencyStore;

#[derive(Clone)]
struct App {
    store: InMemoryIdempotencyStore,
    effects: Arc<AtomicUsize>,
}

/// `POST /things`: creates a thing, counting each real effect. When an
/// `Idempotency-Key` is present it claims it, performs the effect only on
/// `Fresh`, and replays the stored response otherwise — the pattern an
/// application runs inside its `UnitOfWork` transaction with the DORM store.
async fn create(
    State(app): State<App>,
    key: OptionalIdempotencyKey,
    Json(body): Json<Value>,
) -> Result<Response, ProblemResponse> {
    let fp = fingerprint("POST", "/things", None, &body);

    let do_effect = |app: &App| {
        let n = app.effects.fetch_add(1, Ordering::SeqCst) + 1;
        json!({ "created": n, "name": body.get("name").cloned().unwrap_or(Value::Null) })
    };

    let Some(key) = key.0 else {
        // No key: execute normally (idempotency is opt-in per request).
        return Ok((StatusCode::CREATED, Json(do_effect(&app))).into_response());
    };

    let principal = Principal::new("tester", ["things.create"]);
    let sk = scoped_key(&principal, "POST", "/things", &key);
    match app
        .store
        .begin(&sk, &fp, Duration::from_secs(300))
        .await
        .map_err(|_| Problem::internal("idempotency store failed"))?
    {
        IdempotencyOutcome::Fresh => {
            let result = do_effect(&app);
            let bytes = serde_json::to_vec(&result).unwrap();
            app.store
                .finish(&sk, &bytes)
                .await
                .map_err(|_| Problem::internal("idempotency finish failed"))?;
            Ok((StatusCode::CREATED, Json(result)).into_response())
        }
        IdempotencyOutcome::Replay(bytes) => {
            let result: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            Ok((StatusCode::OK, Json(result)).into_response())
        }
        IdempotencyOutcome::Mismatch => Err(mismatch_problem().into()),
        IdempotencyOutcome::InProgress => Err(in_progress_problem().into()),
    }
}

fn app() -> (Router, Arc<AtomicUsize>) {
    let effects = Arc::new(AtomicUsize::new(0));
    let state = App {
        store: InMemoryIdempotencyStore::new(),
        effects: effects.clone(),
    };
    let router = Router::new()
        .route("/things", post(create))
        .with_state(state);
    (router, effects)
}

fn post_thing(key: Option<&str>, body: &str) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri("/things")
        .header("content-type", "application/json");
    if let Some(k) = key {
        b = b.header("idempotency-key", k);
    }
    b.body(Body::from(body.to_owned())).unwrap()
}

async fn status_and_json(res: Response) -> (StatusCode, Value) {
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

#[tokio::test]
async fn same_key_and_body_runs_the_effect_once_and_replays() {
    let (router, effects) = app();

    // First call: Fresh → 201, effect runs.
    let (s1, v1) = status_and_json(
        router
            .clone()
            .oneshot(post_thing(Some("k1"), r#"{"name":"widget"}"#))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(s1, StatusCode::CREATED);
    assert_eq!(v1["created"], 1);
    assert_eq!(effects.load(Ordering::SeqCst), 1);

    // Retry, same key + same body: Replay → the effect does NOT run again.
    let (s2, v2) = status_and_json(
        router
            .oneshot(post_thing(Some("k1"), r#"{"name":"widget"}"#))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(
        v2["created"], 1,
        "the stored result is replayed, not recomputed"
    );
    assert_eq!(effects.load(Ordering::SeqCst), 1, "no second effect");
}

#[tokio::test]
async fn same_key_different_body_is_a_409_and_does_not_execute() {
    let (router, effects) = app();
    router
        .clone()
        .oneshot(post_thing(Some("k1"), r#"{"name":"widget"}"#))
        .await
        .unwrap();
    assert_eq!(effects.load(Ordering::SeqCst), 1);

    let (status, _) = status_and_json(
        router
            .oneshot(post_thing(Some("k1"), r#"{"name":"DIFFERENT"}"#))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        effects.load(Ordering::SeqCst),
        1,
        "a mismatch must not execute"
    );
}

#[tokio::test]
async fn a_request_with_no_key_always_executes() {
    let (router, effects) = app();
    for _ in 0..3 {
        let res = router
            .clone()
            .oneshot(post_thing(None, r#"{"name":"x"}"#))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
    }
    assert_eq!(effects.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_malformed_key_is_rejected_400() {
    let (router, _) = app();
    let res = router
        .oneshot(post_thing(Some("has space"), r#"{"name":"x"}"#))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
