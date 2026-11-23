//! Live end-to-end proof that the ADR 0033 delivery path works over the **DORM**
//! ledger against a real PostgreSQL — the DORM-coupled counterpart of
//! `apps/webhook-delivery-e2e` (which proves the same wire over the in-memory
//! ledger).
//!
//! Two things are proved, in one run against one database:
//!
//! 1. **Atomicity (ADR 0011).** The [`DeliverySubscriber`]'s enqueue and the
//!    inbox claim+mark of [`dispatch`](seiva_events::dispatch) run inside **one**
//!    [`DormUnitOfWork::transaction`]. A transaction that dispatches and then fails
//!    rolls back *both*: no owed delivery row survives, and the inbox key is free
//!    again (a later `begin` is `Fresh`). This is the whole point of the DORM
//!    ledger's ambient-session `enqueue`.
//! 2. **Delivery.** A committed dispatch leaves one `pending` row; one
//!    [`delivery_tick`] claims it, signs it with `seiva-webhooks`, POSTs it over
//!    **real HTTPS** to a local server that VERIFIES the Standard Webhooks
//!    signature, and the [`DormDeliveryLedger`] records `Delivered`, read back from
//!    PostgreSQL.
//!
//! Gated on `SEIVA_DORM_TEST_DSN` (a libpq DSN); skips cleanly when it is unset so
//! the default (offline) build stays green. In CI it runs in the `adapters` job's
//! `dorm` leg, which provides a Postgres service and that env var.
//!
//! The engine's egress guard blocks loopback/`localhost`, so the destination URL
//! uses a public-looking hostname (`webhook.test`) that reqwest resolves
//! statically to `127.0.0.1` — the standard way to drive an SSRF-guarded client at
//! a local server without weakening the guard.

use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use seiva_data_dorm::{
    connect, delivery_migrations, idempotency_migrations, DormConfig, DormDeliveryLedger,
    DormIdempotencyStore, DormUnitOfWork,
};
use seiva_events::{dispatch, SubscriberRegistry};
use seiva_messaging::CloudEvent;
use seiva_ports::{inbox_key, BoxError, IdempotencyOutcome, IdempotencyStore, Scope, UnitOfWork};
use seiva_webhook_delivery::{
    delivery_id, delivery_tick, DeliverySubscriber, InMemorySubscriptions, Subscription,
};
use seiva_webhooks::delivery::{
    DeliveryConfig, DeliveryEngine, DeliveryLedger, DeliveryStatus, HttpRequest, HttpResponse,
    HttpSender, SendError, SigningSecrets,
};
use seiva_webhooks::{
    SignedHeaders, Webhook, DEFAULT_TOLERANCE, ID_HEADER, SIGNATURE_HEADER, TIMESTAMP_HEADER,
};
use tokio::sync::oneshot;

const SECRET: &str = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";
const SUBSCRIPTION: &str = "sub_1";
const EVENT_TYPE: &str = "order.placed";
/// The event that is dispatched then rolled back (atomicity proof).
const EVENT_ROLLBACK: &str = "evt-dorm-rollback";
/// The event that is committed then delivered (delivery proof).
const EVENT_DELIVER: &str = "evt-dorm-deliver";
/// The subscriber id — half of every inbox key (`inbox:webhooks:<event>`).
const SUBSCRIBER: &str = "webhooks";

/// A real [`HttpSender`] over reqwest/TLS — the one seam the engine leaves a port.
/// Identical in spirit to `webhook_delivery_e2e::ReqwestSender`; inlined here so
/// this DORM-coupled crate does not depend on an `apps/*` sibling.
struct ReqwestSender {
    client: reqwest::Client,
}

#[async_trait]
impl HttpSender for ReqwestSender {
    async fn post(&self, request: HttpRequest<'_>) -> Result<HttpResponse, SendError> {
        let response = self
            .client
            .post(request.url)
            .header(ID_HEADER, request.id)
            .header(TIMESTAMP_HEADER, request.timestamp.to_string())
            .header(SIGNATURE_HEADER, request.signature)
            .header(reqwest::header::CONTENT_TYPE, request.content_type)
            .body(request.body.to_vec())
            .send()
            .await
            .map_err(|error| SendError::new(error.to_string()))?;
        let status = response.status().as_u16();
        let body_excerpt = response.text().await.unwrap_or_default();
        Ok(HttpResponse::new(status, body_excerpt))
    }
}

/// A `SigningSecrets` that hands the test secret to the one known subscription.
struct OneSecret;

impl SigningSecrets for OneSecret {
    fn signer(&self, subscription: &str) -> Option<Webhook> {
        (subscription == SUBSCRIPTION).then(|| Webhook::new(SECRET).unwrap())
    }
}

/// Start a local HTTPS server that verifies the Standard Webhooks signature of
/// every POST and signals the first verified delivery.
async fn start_verifying_https_server() -> (u16, oneshot::Receiver<()>) {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert = rcgen::generate_simple_self_signed(vec!["webhook.test".to_string()]).unwrap();
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem(
        cert.cert.pem().into_bytes(),
        cert.key_pair.serialize_pem().into_bytes(),
    )
    .await
    .unwrap();

    let (tx, rx) = oneshot::channel::<()>();
    let tx = Arc::new(std::sync::Mutex::new(Some(tx)));

    let app = Router::new().route(
        "/ingest",
        post(move |headers: HeaderMap, body: Bytes| {
            let tx = Arc::clone(&tx);
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
                        if let Some(tx) = tx.lock().unwrap().take() {
                            let _ = tx.send(());
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

/// Build a DORM config for the test database from the gating DSN.
fn config(dsn: String) -> DormConfig {
    DormConfig {
        dsn,
        replicas: vec![],
        read_only: false,
        schema: None,
        pool_max: 8,
        cursor_key: b"live-webhook-delivery-secret".to_vec(),
    }
}

/// A CloudEvent for the given id — the body is what the subscriber signs and POSTs.
fn event(id: &str) -> CloudEvent {
    CloudEvent::new(
        id,
        "/shop/orders",
        EVENT_TYPE,
        serde_json::json!({ "order": 7, "total": "42.00" }),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dorm_ledger_delivers_and_enqueue_shares_the_inbox_transaction() {
    let Ok(dsn) = std::env::var("SEIVA_DORM_TEST_DSN") else {
        eprintln!("skipping: set SEIVA_DORM_TEST_DSN to run the live DORM delivery e2e");
        return;
    };

    // --- a local HTTPS receiver that verifies the signature of the one delivery.
    let (port, verified) = start_verifying_https_server().await;
    let endpoint = format!("https://webhook.test:{port}/ingest");
    let client = reqwest::Client::builder()
        .resolve("webhook.test", format!("127.0.0.1:{port}").parse().unwrap())
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();

    // --- the DORM boundary: one unit of work for the transactions, one ledger
    // handle for the worker operations (claim/record/get). `enqueue` ignores the
    // ledger's handle and joins the ambient `uow` transaction instead.
    let uow = DormUnitOfWork::new(connect(&config(dsn.clone())).expect("connect uow"));
    let ledger = DormDeliveryLedger::new(connect(&config(dsn)).expect("connect ledger"));

    // --- schema + a clean slate for this run.
    uow.transaction(Scope::Global, || async {
        for stmt in idempotency_migrations()
            .iter()
            .chain(&delivery_migrations())
        {
            dorm_core::db::run(stmt, &[])
                .await
                .map_err(|e| Box::new(e) as BoxError)?;
        }
        dorm_core::db::run(
            "DELETE FROM seiva_webhook_deliveries WHERE subscription = $1",
            &[dorm_core::value::Value::Text(SUBSCRIPTION.to_owned())],
        )
        .await
        .map_err(|e| Box::new(e) as BoxError)?;
        dorm_core::db::run(
            "DELETE FROM dorm_idempotency WHERE key LIKE 'inbox:webhooks:evt-dorm-%'",
            &[],
        )
        .await
        .map_err(|e| Box::new(e) as BoxError)?;
        Ok(())
    })
    .await
    .expect("setup commits");

    // --- the enqueue side: one subscriber over the DORM ledger, resolving one
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

    // =========================================================================
    // PROOF 1 — atomicity: dispatch (inbox claim+mark + enqueue) then FAIL. The
    // whole transaction rolls back, so neither the owed row nor the inbox mark
    // survives.
    // =========================================================================
    let rollback_event = event(EVENT_ROLLBACK);
    let rolled_back = uow
        .transaction(Scope::Global, || async {
            dispatch(&rollback_event, &registry, &DormIdempotencyStore).await?;
            // Everything above is written in this transaction; force a rollback.
            Err::<(), BoxError>("deliberate failure after dispatch".into())
        })
        .await;
    assert!(rolled_back.is_err(), "the dispatching transaction fails");

    // No owed delivery row survived the rollback.
    let rolled_id = delivery_id(SUBSCRIPTION, EVENT_ROLLBACK);
    assert!(
        ledger.get(&rolled_id).await.unwrap().is_none(),
        "a rolled-back dispatch leaves NO owed delivery row"
    );

    // And the inbox key is free again: a later claim is Fresh (had the mark
    // committed, this would be a Replay/InProgress). This single assertion is what
    // proves the enqueue and the inbox mark shared one transaction.
    let inbox_free = uow
        .transaction(Scope::Global, || async {
            DormIdempotencyStore
                .begin(
                    &inbox_key(SUBSCRIBER, EVENT_ROLLBACK),
                    EVENT_ROLLBACK,
                    Duration::from_secs(300),
                )
                .await
        })
        .await
        .expect("the probe begin succeeds");
    assert_eq!(
        inbox_free,
        IdempotencyOutcome::Fresh,
        "the inbox mark rolled back with the enqueue — the key is free"
    );

    // =========================================================================
    // PROOF 2 — delivery: a committed dispatch owes one row; one tick delivers it
    // over real HTTPS and the DORM ledger records Delivered.
    // =========================================================================
    let mut breaker = DeliveryConfig::default().breaker;
    breaker.failure_threshold = 100;
    let engine = DeliveryEngine::new(
        ledger.clone(),
        ReqwestSender { client },
        OneSecret,
        DeliveryConfig {
            post_timeout: Duration::from_secs(10),
            breaker,
            ..Default::default()
        },
    );

    let deliver_event = event(EVENT_DELIVER);
    uow.transaction(Scope::Global, || async {
        dispatch(&deliver_event, &registry, &DormIdempotencyStore).await
    })
    .await
    .expect("the committing dispatch owes the delivery");

    let id = delivery_id(SUBSCRIPTION, EVENT_DELIVER);
    let owed = ledger.get(&id).await.unwrap().expect("the owed row");
    assert_eq!(owed.status, DeliveryStatus::Pending, "owed and pending");
    assert_eq!(owed.attempt, 0);
    assert_eq!(owed.scope, Scope::tenant("acme").unwrap(), "keyed to acme");
    assert_eq!(owed.endpoint, endpoint);

    // One tick claims the one due row and delivers it for real.
    let claimed = delivery_tick(&engine).await.expect("the tick runs");
    assert_eq!(claimed, 1, "the tick claimed the one due row");

    // The receiver actually got a correctly-signed request.
    tokio::time::timeout(Duration::from_secs(5), verified)
        .await
        .expect("the receiver got the delivery within the timeout")
        .expect("the verified signal fired");

    // The DORM ledger records the terminal success, read back from PostgreSQL.
    let row = ledger.get(&id).await.unwrap().expect("the delivered row");
    assert_eq!(
        row.status,
        DeliveryStatus::Delivered,
        "the ledger records Delivered after a 2xx over real HTTPS"
    );
    assert_eq!(row.last_http_status, Some(200));
    assert_eq!(row.attempt, 1, "delivered on the first attempt");

    // --- cleanup (best effort).
    let _ = uow
        .transaction(Scope::Global, || async {
            dorm_core::db::run(
                "DELETE FROM seiva_webhook_deliveries WHERE subscription = $1",
                &[dorm_core::value::Value::Text(SUBSCRIPTION.to_owned())],
            )
            .await
            .map_err(|e| Box::new(e) as BoxError)?;
            dorm_core::db::run(
                "DELETE FROM dorm_idempotency WHERE key LIKE 'inbox:webhooks:evt-dorm-%'",
                &[],
            )
            .await
            .map_err(|e| Box::new(e) as BoxError)?;
            Ok(())
        })
        .await;
}
