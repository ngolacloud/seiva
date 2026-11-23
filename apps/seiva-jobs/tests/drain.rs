//! Live end-to-end proof that the [`seiva_jobs`] composition root drains the DORM
//! delivery ledger **on its own**: a backlog of N owed deliveries is enqueued, the
//! worker is started once via [`seiva_jobs::run_delivery`], and all N rows reach
//! `Delivered` — recorded in PostgreSQL and verified over real HTTPS by the
//! receiver — **without the test ever hand-ticking the engine**. The drain is the
//! [`run_delivery_worker`](seiva_webhook_delivery::run_delivery_worker) loop, not a
//! sequence of `delivery_tick` calls.
//!
//! This is the behaviour the existing single-tick proofs cannot show: the
//! `apps/webhook-delivery-e2e` and `seiva-data-dorm` tests each call `delivery_tick`
//! by hand once; here the composition root's worker claims, delivers and records in
//! a loop until the whole backlog is empty, fast-draining between passes.
//!
//! Gated on `SEIVA_DORM_TEST_DSN` (a libpq DSN); skips cleanly when it is unset so
//! the offline build stays green.
//!
//! The engine's egress guard blocks loopback/`localhost`, so the destination uses a
//! public-looking hostname (`webhook.test`) that reqwest resolves statically to
//! `127.0.0.1` — the standard way to drive an SSRF-guarded client at a local server
//! without weakening the guard.

use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use seiva_data_dorm::{
    connect, delivery_migrations, idempotency_migrations, DormConfig, DormDeliveryLedger,
    DormIdempotencyStore, DormUnitOfWork,
};
use seiva_events::{dispatch, SubscriberRegistry};
use seiva_jobs::{run_delivery, DeliverySettings, ReqwestSender};
use seiva_messaging::CloudEvent;
use seiva_ports::{BoxError, Scope, UnitOfWork};
use seiva_webhook_delivery::{
    delivery_id, DeliverySubscriber, InMemorySubscriptions, Subscription,
};
use seiva_webhooks::delivery::{DeliveryConfig, DeliveryLedger, DeliveryStatus, SigningSecrets};
use seiva_webhooks::{SignedHeaders, Webhook, DEFAULT_TOLERANCE};
use tokio::sync::oneshot;

const SECRET: &str = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";
const SUBSCRIPTION: &str = "sub_drain";
const SUBSCRIBER: &str = "webhooks";
const EVENT_TYPE: &str = "order.placed";
/// The size of the backlog the worker must drain unaided.
const BACKLOG: usize = 5;

/// A `SigningSecrets` that hands the test secret to the one known subscription.
struct OneSecret;

impl SigningSecrets for OneSecret {
    fn signer(&self, subscription: &str) -> Option<Webhook> {
        (subscription == SUBSCRIPTION).then(|| Webhook::new(SECRET).unwrap())
    }
}

/// Start a local HTTPS server that verifies the Standard Webhooks signature of
/// every POST and, once `expected` distinct verified deliveries have arrived,
/// fires the returned signal exactly once. A duplicate delivery id (a redelivery)
/// is counted at most once, so the signal means the whole backlog landed.
async fn start_counting_https_server(expected: usize) -> (u16, oneshot::Receiver<()>) {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert = rcgen::generate_simple_self_signed(vec!["webhook.test".to_string()]).unwrap();
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem(
        cert.cert.pem().into_bytes(),
        cert.key_pair.serialize_pem().into_bytes(),
    )
    .await
    .unwrap();

    let (tx, rx) = oneshot::channel::<()>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let seen: Arc<Mutex<std::collections::HashSet<String>>> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));

    let app = Router::new().route(
        "/ingest",
        post(move |headers: HeaderMap, body: Bytes| {
            let tx = Arc::clone(&tx);
            let seen = Arc::clone(&seen);
            async move {
                let id = headers.get("webhook-id").and_then(|v| v.to_str().ok());
                let ts = headers
                    .get("webhook-timestamp")
                    .and_then(|v| v.to_str().ok());
                let sig = headers
                    .get("webhook-signature")
                    .and_then(|v| v.to_str().ok());
                let (Some(id), Some(ts), Some(sig)) = (id, ts, sig) else {
                    return StatusCode::BAD_REQUEST;
                };
                let wh = Webhook::new(SECRET).unwrap();
                let signed = SignedHeaders {
                    id,
                    timestamp: ts,
                    signature: sig,
                };
                match wh.verify(&signed, &body, SystemTime::now(), DEFAULT_TOLERANCE) {
                    Ok(()) => {
                        // Count each delivery id at most once, then fire the signal
                        // when the whole backlog has been verified.
                        let count = {
                            let mut seen = seen.lock().unwrap();
                            seen.insert(id.to_string());
                            seen.len()
                        };
                        if count == expected {
                            if let Some(tx) = tx.lock().unwrap().take() {
                                let _ = tx.send(());
                            }
                        }
                        StatusCode::OK
                    }
                    Err(_) => StatusCode::BAD_REQUEST,
                }
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum_server::from_tcp_rustls(listener, tls)
            .serve(app.into_make_service())
            .await
            .unwrap();
    });

    (port, rx)
}

/// A DORM config for the test database from the gating DSN.
fn config(dsn: String) -> DormConfig {
    DormConfig {
        dsn,
        replicas: vec![],
        read_only: false,
        schema: None,
        pool_max: 8,
        cursor_key: b"seiva-jobs-drain-secret".to_vec(),
    }
}

/// A CloudEvent for the given id — the body the subscriber signs and POSTs.
fn event(id: &str) -> CloudEvent {
    CloudEvent::new(
        id,
        "/shop/orders",
        EVENT_TYPE,
        serde_json::json!({ "order": 7, "total": "42.00" }),
    )
}

/// The event ids of the backlog (`evt-drain-0` .. `evt-drain-N`).
fn backlog_event_ids() -> Vec<String> {
    (0..BACKLOG).map(|i| format!("evt-drain-{i}")).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_worker_drains_a_backlog_on_its_own() {
    let Ok(dsn) = std::env::var("SEIVA_DORM_TEST_DSN") else {
        eprintln!("skipping: set SEIVA_DORM_TEST_DSN to run the live drain e2e");
        return;
    };

    // --- a local HTTPS receiver that verifies each delivery and signals at BACKLOG.
    let (port, all_delivered) = start_counting_https_server(BACKLOG).await;
    let endpoint = format!("https://webhook.test:{port}/ingest");
    let client = reqwest::Client::builder()
        .resolve("webhook.test", format!("127.0.0.1:{port}").parse().unwrap())
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();

    // --- the DORM boundary: one unit of work for the enqueue/setup transactions,
    // one ledger handle shared by the subscriber, the read-backs, and the worker.
    let uow = DormUnitOfWork::new(connect(&config(dsn.clone())).expect("connect uow"));
    let ledger = DormDeliveryLedger::new(connect(&config(dsn)).expect("connect ledger"));

    // --- schema + a clean slate for this run.
    uow.transaction(Scope::Global, || async {
        for stmt in idempotency_migrations().iter().chain(&delivery_migrations()) {
            dorm_core::db::run(stmt, &[])
                .await
                .map_err(|e| Box::new(e) as BoxError)?;
        }
        clean_slate().await
    })
    .await
    .expect("setup commits");

    // --- the enqueue side: one subscriber over the DORM ledger, resolving the one
    // subscription for `acme` to the local endpoint.
    let mut subscriptions = InMemorySubscriptions::new();
    subscriptions.register(
        EVENT_TYPE,
        Subscription {
            id: SUBSCRIPTION.to_string(),
            endpoint: endpoint.clone(),
            scope: Scope::tenant("acme").unwrap(),
        },
    );
    let subscriber = DeliverySubscriber::new(SUBSCRIBER, ledger.clone(), subscriptions);
    let mut registry = SubscriberRegistry::new();
    registry.register(EVENT_TYPE, Arc::new(subscriber));

    // --- enqueue the whole backlog in ONE committed transaction: N distinct events
    // ⇒ N owed `pending` rows, each due now (`next_attempt_at = epoch`).
    let ids = backlog_event_ids();
    uow.transaction(Scope::Global, || async {
        for id in &ids {
            dispatch(&event(id), &registry, &DormIdempotencyStore).await?;
        }
        Ok(())
    })
    .await
    .expect("the committing dispatch owes the whole backlog");

    // The backlog is owed and pending before the worker starts.
    for id in &ids {
        let row = ledger
            .get(&delivery_id(SUBSCRIPTION, id))
            .await
            .unwrap()
            .expect("an owed row");
        assert_eq!(row.status, DeliveryStatus::Pending, "owed and pending");
        assert_eq!(row.attempt, 0);
    }

    // =========================================================================
    // Start the worker ONCE and let it drain. No `delivery_tick` is called here.
    // =========================================================================
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    // A tolerant breaker + generous timeout so the happy path is what is proven;
    // a tiny interval + fast-drain-at-1 so the backlog empties with no idle waits.
    let mut breaker = DeliveryConfig::default().breaker;
    breaker.failure_threshold = 100;
    let settings = DeliverySettings {
        worker_name: "seiva-jobs-drain-test".to_string(),
        interval: Duration::from_millis(50),
        drain_threshold: 1,
        delivery: DeliveryConfig {
            post_timeout: Duration::from_secs(10),
            breaker,
            ..Default::default()
        },
    };
    let worker = tokio::spawn(run_delivery(
        ledger.clone(),
        ReqwestSender::new(client),
        OneSecret,
        settings,
        async move {
            let _ = shutdown_rx.await;
        },
    ));

    // The receiver verified ALL N deliveries — the worker drained the backlog on
    // its own, without a single hand-rolled tick.
    tokio::time::timeout(Duration::from_secs(30), all_delivered)
        .await
        .expect("the worker delivered the whole backlog within the timeout")
        .expect("the all-delivered signal fired");

    // The DORM ledger records every row terminal-success, read back from Postgres.
    // The receiver's signal fires the instant the last POST is VERIFIED, a moment
    // before the worker's `record(Delivered)` lands — so poll each row to its
    // terminal state (eventually consistent within a few ms) rather than racing it.
    for id in &ids {
        let did = delivery_id(SUBSCRIPTION, id);
        let mut row = ledger.get(&did).await.unwrap().expect("the delivered row");
        for _ in 0..200 {
            if row.status == DeliveryStatus::Delivered {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
            row = ledger.get(&did).await.unwrap().expect("the delivered row");
        }
        assert_eq!(
            row.status,
            DeliveryStatus::Delivered,
            "every backlog row records Delivered after a 2xx over real HTTPS"
        );
        assert_eq!(row.last_http_status, Some(200));
        assert_eq!(row.attempt, 1, "delivered on the first attempt");
    }

    // --- stop the worker and collect its report: it ran its own loop and
    // processed at least the whole backlog, with no hand-ticking.
    let _ = shutdown_tx.send(());
    let report = tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .expect("the worker stops promptly after shutdown")
        .expect("the worker task did not panic");
    assert!(report.passes >= 1, "the worker ran at least one pass");
    assert!(
        report.processed >= BACKLOG as u64,
        "the worker claimed at least the whole backlog ({} >= {BACKLOG})",
        report.processed,
    );

    // --- cleanup (best effort).
    let _ = uow
        .transaction(Scope::Global, || async { clean_slate().await })
        .await;
}

/// Delete this run's delivery rows and inbox marks (used for setup and teardown).
/// Runs on the ambient transaction opened by the caller.
async fn clean_slate() -> Result<(), BoxError> {
    dorm_core::db::run(
        "DELETE FROM seiva_webhook_deliveries WHERE subscription = $1",
        &[dorm_core::value::Value::Text(SUBSCRIPTION.to_owned())],
    )
    .await
    .map_err(|e| Box::new(e) as BoxError)?;
    dorm_core::db::run(
        "DELETE FROM dorm_idempotency WHERE key LIKE 'inbox:webhooks:evt-drain-%'",
        &[],
    )
    .await
    .map_err(|e| Box::new(e) as BoxError)?;
    Ok(())
}
