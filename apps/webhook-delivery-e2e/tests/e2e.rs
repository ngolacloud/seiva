//! End-to-end proof that the ADR 0033 delivery engine, wired to the runtime,
//! delivers a signed webhook over real HTTPS.
//!
//! The whole wire, no doubles on the hot path except the ledger:
//!
//! 1. a [`DeliverySubscriber`] is registered on the event bus and a domain event
//!    is pushed through [`seiva_events::dispatch`] — the subscriber enqueues one
//!    owed [`Delivery`] into the ledger (the ADR 0021 enqueue side);
//! 2. one [`delivery_tick`] (the ADR 0034 job) claims the row, signs it with
//!    `seiva-webhooks`, validates egress, and POSTs it through a **real**
//!    [`ReqwestSender`] to a **local HTTPS** server;
//! 3. the server VERIFIES the Standard Webhooks signature and answers `200`;
//! 4. the ledger row is observed `Delivered`.
//!
//! The engine's egress guard blocks loopback/`localhost`, so the destination URL
//! uses a public-looking hostname (`webhook.test`) that reqwest is told to resolve
//! statically to `127.0.0.1` — the standard way to drive an SSRF-guarded client at
//! a local server without weakening the guard.

use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use seiva_events::{dispatch, SubscriberRegistry};
use seiva_messaging::CloudEvent;
use seiva_ports::Scope;
use seiva_testing::InMemoryIdempotencyStore;
use seiva_webhook_delivery::{
    delivery_id, delivery_tick, DeliverySubscriber, InMemorySubscriptions, Subscription,
};
use seiva_webhooks::delivery::{
    DeliveryConfig, DeliveryEngine, DeliveryLedger, DeliveryStatus, InMemoryLedger, SigningSecrets,
};
use seiva_webhooks::{SignedHeaders, Webhook, DEFAULT_TOLERANCE};
use tokio::sync::oneshot;
use webhook_delivery_e2e::ReqwestSender;

const SECRET: &str = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";
const SUBSCRIPTION: &str = "sub_1";
const EVENT_TYPE: &str = "order.placed";
const EVENT_ID: &str = "evt-e2e-1";

/// A `SigningSecrets` that hands the test secret to the one known subscription.
struct OneSecret;

impl SigningSecrets for OneSecret {
    fn signer(&self, subscription: &str) -> Option<Webhook> {
        (subscription == SUBSCRIPTION).then(|| Webhook::new(SECRET).unwrap())
    }
}

/// Start a local HTTPS server that verifies the Standard Webhooks signature of
/// every POST and signals the first verified delivery. Returns the bound port and
/// a receiver that fires once a correctly-signed request has arrived.
async fn start_verifying_https_server() -> (u16, oneshot::Receiver<()>) {
    // rustls 0.23 needs a process-wide provider before a ServerConfig is built.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // A self-signed certificate for the hostname the engine will POST to. The
    // client accepts invalid certs (a test), so only the handshake matters.
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
                        // Fire the "delivered-and-verified" signal exactly once.
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

    // Bind a std listener to an ephemeral port so we learn the port before serving.
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_published_event_is_delivered_over_real_https_and_recorded() {
    let (port, verified) = start_verifying_https_server().await;

    // `webhook.test` passes the egress guard (not localhost, not an IP literal);
    // reqwest resolves it to the local server's loopback address + port. The
    // destination URL carries the real port so the connect targets it.
    let endpoint = format!("https://webhook.test:{port}/ingest");
    let client = reqwest::Client::builder()
        .resolve("webhook.test", format!("127.0.0.1:{port}").parse().unwrap())
        // A self-signed test certificate: skip verification on the client side.
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();

    // The ledger (the durable source of truth) and the engine over the REAL sender.
    let ledger = InMemoryLedger::new();
    // Generous timeout; a tolerant breaker (this proves the happy path end-to-end).
    let mut breaker = DeliveryConfig::default().breaker;
    breaker.failure_threshold = 100;
    let config = DeliveryConfig {
        post_timeout: Duration::from_secs(10),
        breaker,
        ..Default::default()
    };
    let engine = DeliveryEngine::new(
        ledger.clone(),
        ReqwestSender::new(client),
        OneSecret,
        config,
    );

    // --- enqueue side (ADR 0021): the subscriber turns the event into an owed row.
    let mut subscriptions = InMemorySubscriptions::new();
    subscriptions.register(
        EVENT_TYPE,
        Subscription {
            id: SUBSCRIPTION.to_string(),
            endpoint: endpoint.clone(),
            scope: Scope::tenant("acme").unwrap(),
        },
    );
    let subscriber = DeliverySubscriber::new("webhooks", ledger.clone(), subscriptions);
    let mut registry = SubscriberRegistry::new();
    registry.register(EVENT_TYPE, Arc::new(subscriber));

    let event = CloudEvent::new(
        EVENT_ID,
        "/shop/orders",
        EVENT_TYPE,
        serde_json::json!({ "order": 7, "total": "42.00" }),
    );
    dispatch(&event, &registry, &InMemoryIdempotencyStore::new())
        .await
        .expect("the subscriber enqueues the owed delivery");

    let id = delivery_id(SUBSCRIPTION, EVENT_ID);
    // The row is owed and pending before any tick.
    assert_eq!(
        ledger.get(&id).await.unwrap().unwrap().status,
        DeliveryStatus::Pending,
        "the event is enqueued as owed"
    );

    // --- send side (ADR 0034): one tick claims the row and delivers it for real.
    let claimed = delivery_tick(&engine).await.expect("the tick runs");
    assert_eq!(claimed, 1, "the tick claimed the one due row");

    // The receiver actually got a correctly-signed request.
    tokio::time::timeout(Duration::from_secs(5), verified)
        .await
        .expect("the receiver got the delivery within the timeout")
        .expect("the verified signal fired");

    // --- the ledger records the terminal success.
    let row = ledger.get(&id).await.unwrap().unwrap();
    assert_eq!(
        row.status,
        DeliveryStatus::Delivered,
        "the ledger records Delivered after a 2xx over real HTTPS"
    );
    assert_eq!(row.last_http_status, Some(200));
    assert_eq!(row.attempt, 1, "delivered on the first attempt");
}
