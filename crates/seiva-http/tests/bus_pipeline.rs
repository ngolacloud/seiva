//! The ADR 0020 pipeline, proven as a **composition** (Phase 2).
//!
//! These tests wire the full write chain
//! `Authorize → Validate → Transactional{ Idempotent → handler }` around a request
//! [`Ctx`](seiva_http::Ctx), using the in-memory port doubles, and assert the
//! behaviors interact as the ADR requires:
//!
//! - a caller without the required permission is denied *before* the handler runs,
//!   and no transaction opens (authz fail-closed, pre-transaction);
//! - an invalid command is rejected before the handler runs (validation,
//!   pre-transaction);
//! - a retry with the same `Idempotency-Key` produces exactly one effect (the
//!   handler runs once, the second dispatch replays);
//! - a key reused with different content is a status-bearing `409` that never runs
//!   the handler (idempotency mismatch);
//! - a failing handler rolls the transaction back.
//!
//! There is **no** live HTTP server and no example module here — ADR 0020 Phase 2
//! proves the composition; routing a real module (orders) through the bus is
//! Phase 3.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use seiva_cqrs::{Command, CommandHandler, Mediator, Transactional, Validate, Validated};
use seiva_http::bus::{Authorize, Idempotent};
use seiva_http::{Ctx, IdempotencyKey, Problem, RequiredPermission};
use seiva_identity::Principal;
use seiva_ports::PortResult;
use seiva_testing::{InMemoryIdempotencyStore, InMemoryUnitOfWork, TxOutcome};

// ----- The use case under test ---------------------------------------------

/// A write command: place an order. Well-formed only when `qty` is positive, so a
/// test can dispatch both a valid and an invalid instance through `Validate`.
#[derive(Clone)]
struct PlaceOrder {
    sku: String,
    qty: u32,
}

impl Command for PlaceOrder {
    type Output = OrderPlaced;
}

impl PlaceOrder {
    /// The request body a transport would carry for this command — the bytes
    /// `Idempotent` fingerprints, so logically-different content (a different
    /// `sku`/`qty`) yields a different fingerprint.
    fn body(&self) -> serde_json::Value {
        serde_json::json!({ "sku": self.sku, "qty": self.qty })
    }
}

impl Validated for PlaceOrder {
    fn validate(&self) -> PortResult<()> {
        if self.qty == 0 {
            return Err("qty must be positive".into());
        }
        Ok(())
    }
}

/// The command's output: serialisable so `Idempotent` can store and replay it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct OrderPlaced {
    order_id: u64,
}

/// A handler that records how many times it actually ran, so a test can prove a
/// behavior short-circuited (0 runs) or replayed (1 run across two dispatches).
struct PlaceOrderHandler {
    runs: Arc<AtomicU64>,
    next_id: Arc<AtomicU64>,
}

impl PlaceOrderHandler {
    fn new(runs: Arc<AtomicU64>) -> Self {
        Self {
            runs,
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl CommandHandler<PlaceOrder> for PlaceOrderHandler {
    async fn handle(&self, _cmd: PlaceOrder) -> PortResult<OrderPlaced> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        Ok(OrderPlaced {
            order_id: self.next_id.fetch_add(1, Ordering::SeqCst),
        })
    }
}

/// A handler that always fails *after* recording it ran, to prove the transaction
/// rolls back on an inner error.
struct FailingHandler {
    runs: Arc<AtomicU64>,
}

impl CommandHandler<PlaceOrder> for FailingHandler {
    async fn handle(&self, _cmd: PlaceOrder) -> PortResult<OrderPlaced> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        Err("inventory service unavailable".into())
    }
}

// ----- Fixtures ------------------------------------------------------------

const PLACE: RequiredPermission = RequiredPermission("orders.place");

fn a_command() -> PlaceOrder {
    PlaceOrder {
        sku: "SKU-1".to_owned(),
        qty: 2,
    }
}

fn a_body() -> serde_json::Value {
    a_command().body()
}

/// A request context for `principal` carrying the operation coordinates and
/// `idempotency_key` — exactly what the transport injects, built here by hand.
fn ctx_for(principal: Principal, idempotency_key: Option<&str>) -> Ctx {
    let key = idempotency_key.map(|raw| IdempotencyKey::parse(raw).expect("valid test key"));
    Ctx::new(principal).with_request("POST", "/v1/orders", key)
}

/// The status a port error maps to at the HTTP edge — exercises the downcast-aware
/// `From<BoxError> for Problem`, proving the behavior's status is **not remapped**.
fn status_of(err: seiva_ports::BoxError) -> StatusCode {
    Problem::from(err).status
}

// ----- Tests ---------------------------------------------------------------

#[tokio::test]
async fn an_unauthorized_caller_is_denied_and_the_handler_never_runs() {
    let runs = Arc::new(AtomicU64::new(0));
    let handler = PlaceOrderHandler::new(runs.clone());
    let uow = InMemoryUnitOfWork::new();
    let store = InMemoryIdempotencyStore::new();

    // A caller in the right tenant but WITHOUT `orders.place`.
    let ctx = ctx_for(
        Principal::in_tenant("mallory", "acme", ["orders.read"]),
        Some("key-1"),
    );
    let idem = Idempotent::from_ctx(handler, store, &ctx, &a_body(), Duration::from_secs(60))
        .expect("ctx carries an idempotency key");
    let chain = Authorize::from_ctx(
        Validate::new(Transactional::new(idem, uow.clone())),
        &ctx,
        PLACE,
    );
    let mediator = Mediator::new(chain);

    let err = mediator.send(a_command()).await.unwrap_err();

    // Fail-closed: a 403, the handler never ran, and (pre-transaction) no tx opened.
    assert_eq!(status_of(err), StatusCode::FORBIDDEN);
    assert_eq!(runs.load(Ordering::SeqCst), 0, "handler must not run");
    assert!(
        uow.outcomes().is_empty(),
        "authz is pre-transaction: no transaction may open for a denied call"
    );
}

#[tokio::test]
async fn an_invalid_command_is_rejected_before_the_handler() {
    let runs = Arc::new(AtomicU64::new(0));
    let handler = PlaceOrderHandler::new(runs.clone());
    let uow = InMemoryUnitOfWork::new();
    let store = InMemoryIdempotencyStore::new();

    // Authorized caller, but a malformed command (qty = 0).
    let ctx = ctx_for(
        Principal::in_tenant("alice", "acme", ["orders.place"]),
        Some("key-1"),
    );
    let idem = Idempotent::from_ctx(handler, store, &ctx, &a_body(), Duration::from_secs(60))
        .expect("ctx carries an idempotency key");
    let chain = Authorize::from_ctx(
        Validate::new(Transactional::new(idem, uow.clone())),
        &ctx,
        PLACE,
    );
    let mediator = Mediator::new(chain);

    let err = mediator
        .send(PlaceOrder {
            sku: "SKU-1".to_owned(),
            qty: 0,
        })
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "qty must be positive");
    assert_eq!(runs.load(Ordering::SeqCst), 0, "handler must not run");
    assert!(
        uow.outcomes().is_empty(),
        "validation is pre-transaction: no transaction may open for an invalid call"
    );
}

#[tokio::test]
async fn a_retry_with_the_same_key_produces_one_effect() {
    // Shared, durable state across the two requests: the idempotency store, the
    // unit of work, and the handler's run counter. Each request builds its own
    // per-request behavior chain around that shared state, as a real server would.
    let runs = Arc::new(AtomicU64::new(0));
    let store = InMemoryIdempotencyStore::new();
    let uow = InMemoryUnitOfWork::new();
    let ctx = ctx_for(
        Principal::in_tenant("alice", "acme", ["orders.place"]),
        Some("retry-key"),
    );

    let dispatch = |ctx: &Ctx| {
        let handler = PlaceOrderHandler::new(runs.clone());
        let idem = Idempotent::from_ctx(
            handler,
            store.clone(),
            ctx,
            &a_body(),
            Duration::from_secs(60),
        )
        .expect("ctx carries an idempotency key");
        let chain = Authorize::from_ctx(
            Validate::new(Transactional::new(idem, uow.clone())),
            ctx,
            PLACE,
        );
        Mediator::new(chain)
    };

    let first = dispatch(&ctx).send(a_command()).await.unwrap();
    let second = dispatch(&ctx).send(a_command()).await.unwrap();

    // The handler ran exactly once; the retry replayed the stored response.
    assert_eq!(runs.load(Ordering::SeqCst), 1, "one effect only");
    assert_eq!(first, second, "the retry replays the first response");
    // Both transactions committed (the replay commits an empty, effect-free scope).
    assert_eq!(uow.outcomes(), [TxOutcome::Committed, TxOutcome::Committed]);
}

#[tokio::test]
async fn the_same_key_with_different_content_is_a_status_bearing_conflict() {
    let runs = Arc::new(AtomicU64::new(0));
    let store = InMemoryIdempotencyStore::new();
    let uow = InMemoryUnitOfWork::new();
    let principal = Principal::in_tenant("alice", "acme", ["orders.place"]);
    let ctx = ctx_for(principal, Some("same-key"));

    // First request: fresh, runs once.
    let handler = PlaceOrderHandler::new(runs.clone());
    let idem = Idempotent::from_ctx(
        handler,
        store.clone(),
        &ctx,
        &a_body(),
        Duration::from_secs(60),
    )
    .expect("ctx carries an idempotency key");
    let chain = Authorize::from_ctx(
        Validate::new(Transactional::new(idem, uow.clone())),
        &ctx,
        PLACE,
    );
    Mediator::new(chain).send(a_command()).await.unwrap();

    // Second request: SAME key, DIFFERENT body → fingerprint mismatch → 409.
    let other_body = serde_json::json!({ "sku": "SKU-1", "qty": 99 });
    let handler2 = PlaceOrderHandler::new(runs.clone());
    let idem2 = Idempotent::from_ctx(
        handler2,
        store.clone(),
        &ctx,
        &other_body,
        Duration::from_secs(60),
    )
    .expect("ctx carries an idempotency key");
    let chain2 = Authorize::from_ctx(
        Validate::new(Transactional::new(idem2, uow.clone())),
        &ctx,
        PLACE,
    );
    let err = Mediator::new(chain2)
        .send(PlaceOrder {
            sku: "SKU-1".to_owned(),
            qty: 99,
        })
        .await
        .unwrap_err();

    // Status-bearing: recovered as a 409, not flattened to 500.
    assert_eq!(status_of(err), StatusCode::CONFLICT);
    // Only the first request ran the handler; the mismatch never executed.
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_failing_handler_rolls_the_transaction_back() {
    let runs = Arc::new(AtomicU64::new(0));
    let handler = FailingHandler { runs: runs.clone() };
    let uow = InMemoryUnitOfWork::new();
    let store = InMemoryIdempotencyStore::new();

    let ctx = ctx_for(
        Principal::in_tenant("alice", "acme", ["orders.place"]),
        Some("key-1"),
    );
    let idem = Idempotent::from_ctx(handler, store, &ctx, &a_body(), Duration::from_secs(60))
        .expect("ctx carries an idempotency key");
    let chain = Authorize::from_ctx(
        Validate::new(Transactional::new(idem, uow.clone())),
        &ctx,
        PLACE,
    );
    let mediator = Mediator::new(chain);

    let err = mediator.send(a_command()).await.unwrap_err();

    assert_eq!(err.to_string(), "inventory service unavailable");
    assert_eq!(
        runs.load(Ordering::SeqCst),
        1,
        "the handler ran and then failed"
    );
    // The inner error rolled the transaction back (observed through the UoW).
    assert_eq!(uow.outcomes(), [TxOutcome::RolledBack]);
}
