//! Integration tests for the ADR 0033 delivery engine: the queue/claim,
//! retry-with-backoff, dead-letter after the attempt cap, the per-endpoint
//! circuit breaker, egress validation, the idempotency key, and the deliveries
//! ledger as the durable source of truth.
//!
//! The engine is driven with an injected `now`, so every timing assertion is
//! deterministic without a real wall-clock wait. The one real timer — the
//! per-POST `timeout` — is exercised under a paused tokio clock.

#![cfg(feature = "delivery")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use seiva_ports::Scope;
use seiva_resilience::{BreakerConfig, RetryPolicy};
use seiva_webhooks::delivery::{
    Delivery, DeliveryConfig, DeliveryEngine, DeliveryId, DeliveryLedger, DeliveryStatus,
    HttpRequest, HttpResponse, InMemoryLedger, NewDelivery, SendError, SigningSecrets,
};
use seiva_webhooks::{SignedHeaders, Webhook, DEFAULT_TOLERANCE};

const SECRET: &str = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";
const ENDPOINT: &str = "https://hooks.example.com/ingest";
const PAYLOAD: &[u8] = br#"{"event":"order.placed","data":{"id":1}}"#;

// ---------- test doubles ----------

/// A `SigningSecrets` that hands the same secret to every known subscription and
/// `None` to an unknown one.
struct OneSecret {
    secret: String,
    known: &'static str,
}

impl SigningSecrets for OneSecret {
    fn signer(&self, subscription: &str) -> Option<Webhook> {
        if subscription == self.known {
            Some(Webhook::new(&self.secret).unwrap())
        } else {
            None
        }
    }
}

/// One scripted response (or a transport error, or a hang that outlives the POST
/// timeout) per `post` call, in order; the calls are also counted and the last
/// request captured for signature assertions.
#[derive(Clone)]
struct ScriptedSender {
    script: Arc<Mutex<std::collections::VecDeque<Reply>>>,
    calls: Arc<AtomicUsize>,
    last: Arc<Mutex<Option<CapturedRequest>>>,
}

#[derive(Clone)]
enum Reply {
    Respond(u16, String),
    Transport(String),
    /// Sleep longer than any sane POST timeout, so `timeout` cancels it.
    Hang,
}

#[derive(Clone)]
struct CapturedRequest {
    id: String,
    timestamp: i64,
    signature: String,
    body: Vec<u8>,
}

impl ScriptedSender {
    fn new(replies: impl IntoIterator<Item = Reply>) -> Self {
        Self {
            script: Arc::new(Mutex::new(replies.into_iter().collect())),
            calls: Arc::new(AtomicUsize::new(0)),
            last: Arc::new(Mutex::new(None)),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn last_request(&self) -> Option<CapturedRequest> {
        self.last.lock().unwrap().clone()
    }
}

#[async_trait]
impl seiva_webhooks::delivery::HttpSender for ScriptedSender {
    async fn post(&self, request: HttpRequest<'_>) -> Result<HttpResponse, SendError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last.lock().unwrap() = Some(CapturedRequest {
            id: request.id.to_string(),
            timestamp: request.timestamp,
            signature: request.signature.to_string(),
            body: request.body.to_vec(),
        });
        let reply = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            // Default once the script runs dry: keep succeeding.
            .unwrap_or(Reply::Respond(200, "ok".to_string()));
        match reply {
            Reply::Respond(status, body) => Ok(HttpResponse::new(status, body)),
            Reply::Transport(msg) => Err(SendError::new(msg)),
            Reply::Hang => {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                Ok(HttpResponse::new(200, "unreachable"))
            }
        }
    }
}

// ---------- helpers ----------

fn secrets() -> OneSecret {
    OneSecret {
        secret: SECRET.to_string(),
        known: "sub_1",
    }
}

fn enqueued(id: &str) -> NewDelivery {
    NewDelivery {
        id: DeliveryId::new(id),
        subscription: "sub_1".to_string(),
        event_id: "evt_1".to_string(),
        endpoint: ENDPOINT.to_string(),
        payload: PAYLOAD.to_vec(),
        scope: Scope::tenant("acme"),
    }
}

/// `now`, `now + secs`.
fn at(base: SystemTime, secs: u64) -> SystemTime {
    base + Duration::from_secs(secs)
}

async fn row(ledger: &InMemoryLedger, id: &str) -> Delivery {
    ledger
        .get(&DeliveryId::new(id))
        .await
        .unwrap()
        .expect("row exists")
}

// A config with a fast, bounded schedule for the tests.
fn config(max_attempts: u32) -> DeliveryConfig {
    DeliveryConfig {
        retry: RetryPolicy {
            max_attempts,
            backoff_base_ms: 1000,
        },
        post_timeout: Duration::from_millis(100),
        breaker: BreakerConfig {
            failure_threshold: 1,
            open_cooldown: Duration::from_secs(30),
            half_open_max: 1,
            success_threshold: 1,
        },
        lease: Duration::from_secs(30),
        batch: 16,
        content_type: "application/json".to_string(),
        excerpt_limit: 64,
    }
}

// ---------- the ledger: enqueue / claim / fail-closed ----------

#[tokio::test]
async fn enqueue_is_idempotent_on_the_delivery_id() {
    let ledger = InMemoryLedger::new();
    ledger.enqueue(enqueued("msg_1")).await.unwrap();
    // The same id again leaves a single row.
    ledger.enqueue(enqueued("msg_1")).await.unwrap();
    assert_eq!(ledger.len(), 1);

    let d = row(&ledger, "msg_1").await;
    assert_eq!(d.status, DeliveryStatus::Pending);
    assert_eq!(d.attempt, 0);
}

#[tokio::test]
async fn enqueue_without_a_scope_fails_closed() {
    let ledger = InMemoryLedger::new();
    let mut new = enqueued("msg_1");
    new.scope = None;
    let err = ledger.enqueue(new).await.unwrap_err();
    assert_eq!(
        err,
        seiva_webhooks::delivery::LedgerError::MissingScope,
        "a tenant-less delivery is refused, never recorded"
    );
    assert!(ledger.is_empty());
}

#[tokio::test]
async fn claim_due_leases_a_row_so_a_second_claim_skips_it() {
    let ledger = InMemoryLedger::new();
    ledger.enqueue(enqueued("msg_1")).await.unwrap();
    let now = at(UNIX_EPOCH, 1_000);

    let first = ledger
        .claim_due(now, 16, Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].status, DeliveryStatus::InFlight);

    // A second claim within the lease window sees nothing.
    let second = ledger
        .claim_due(now, 16, Duration::from_secs(30))
        .await
        .unwrap();
    assert!(second.is_empty(), "the lease hides the in-flight row");

    // Once the lease lapses, the orphan is reclaimable (crash recovery).
    let later = at(now, 31);
    let third = ledger
        .claim_due(later, 16, Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(
        third.len(),
        1,
        "an expired lease makes the orphan claimable"
    );
}

// ---------- happy path: deliver, sign, and prove the idempotency key ----------

#[tokio::test]
async fn a_delivered_webhook_is_signed_with_its_id_as_the_idempotency_key() {
    let ledger = InMemoryLedger::new();
    ledger.enqueue(enqueued("msg_1")).await.unwrap();
    let sender = ScriptedSender::new([Reply::Respond(200, "ok".to_string())]);
    let engine = DeliveryEngine::new(ledger.clone(), sender.clone(), secrets(), config(5));

    let now = at(UNIX_EPOCH, 1_700_000_000);
    let report = engine.tick(now).await.unwrap();
    assert_eq!(report.claimed, 1);
    assert_eq!(report.delivered, 1);

    let d = row(&ledger, "msg_1").await;
    assert_eq!(d.status, DeliveryStatus::Delivered);
    assert_eq!(d.attempt, 1);
    assert_eq!(d.last_http_status, Some(200));

    // The captured request carries the delivery id as webhook-id, and its
    // signature verifies under the subscription's secret — the receiver can use
    // the same id to deduplicate (at-least-once + idempotency).
    let req = sender.last_request().unwrap();
    assert_eq!(req.id, "msg_1");
    let wh = Webhook::new(SECRET).unwrap();
    let headers = SignedHeaders {
        id: &req.id,
        timestamp: &req.timestamp.to_string(),
        signature: &req.signature,
    };
    wh.verify(&headers, &req.body, now, DEFAULT_TOLERANCE)
        .expect("the delivered payload verifies under the signing secret");
}

// ---------- retry with backoff, then dead-letter after the cap ----------

#[tokio::test]
async fn a_transient_failure_retries_with_backoff_then_dead_letters_after_the_cap() {
    let ledger = InMemoryLedger::new();
    ledger.enqueue(enqueued("msg_1")).await.unwrap();
    // Every attempt is a 503 (retryable); the cap is three attempts.
    let sender = ScriptedSender::new([
        Reply::Respond(503, "unavailable".to_string()),
        Reply::Respond(503, "unavailable".to_string()),
        Reply::Respond(503, "unavailable".to_string()),
    ]);
    // Breaker tolerant enough not to trip during this test (we are proving the
    // retry/dead-letter path, not the breaker).
    let mut cfg = config(3);
    cfg.breaker.failure_threshold = 100;
    let engine = DeliveryEngine::new(ledger.clone(), sender.clone(), secrets(), cfg);

    // Attempt 1 at t0 → reschedule.
    let t0 = at(UNIX_EPOCH, 1_000);
    let r1 = engine.tick(t0).await.unwrap();
    assert_eq!((r1.retried, r1.dead), (1, 0));
    let d = row(&ledger, "msg_1").await;
    assert_eq!(d.status, DeliveryStatus::Pending);
    assert_eq!(d.attempt, 1);
    // Backoff after attempt 1 is base × 2 = 2000 ms (the shared RelayPolicy schedule).
    assert_eq!(d.next_attempt_at, t0 + Duration::from_millis(2000));

    // Before the backoff elapses the row is NOT due: a tick claims nothing.
    let early = at(t0, 1); // +1s, still < 2s backoff
    let r_early = engine.tick(early).await.unwrap();
    assert_eq!(
        r_early.claimed, 0,
        "a rescheduled row is not due before its backoff"
    );
    assert_eq!(sender.call_count(), 1, "no second POST before the backoff");

    // Attempt 2 once due → reschedule again (backoff base × 4 = 4000 ms).
    let t1 = t0 + Duration::from_millis(2000);
    let r2 = engine.tick(t1).await.unwrap();
    assert_eq!((r2.retried, r2.dead), (1, 0));
    let d = row(&ledger, "msg_1").await;
    assert_eq!(d.attempt, 2);
    assert_eq!(d.next_attempt_at, t1 + Duration::from_millis(4000));

    // Attempt 3 reaches the cap → dead-letter.
    let t2 = t1 + Duration::from_millis(4000);
    let r3 = engine.tick(t2).await.unwrap();
    assert_eq!((r3.retried, r3.dead), (0, 1));
    let d = row(&ledger, "msg_1").await;
    assert_eq!(
        d.status,
        DeliveryStatus::Dead,
        "dead-lettered after the attempt cap"
    );
    assert_eq!(d.attempt, 3);
    assert_eq!(d.last_http_status, Some(503));
    assert_eq!(sender.call_count(), 3, "exactly max_attempts POSTs");
}

// ---------- permanent HTTP rejection is not retried ----------

#[tokio::test]
async fn a_non_retryable_client_error_fails_permanently_without_retrying() {
    let ledger = InMemoryLedger::new();
    ledger.enqueue(enqueued("msg_1")).await.unwrap();
    let sender = ScriptedSender::new([Reply::Respond(404, "no such hook".to_string())]);
    let mut cfg = config(5);
    cfg.breaker.failure_threshold = 100;
    let engine = DeliveryEngine::new(ledger.clone(), sender.clone(), secrets(), cfg);

    let report = engine.tick(at(UNIX_EPOCH, 1_000)).await.unwrap();
    assert_eq!(report.failed, 1);
    let d = row(&ledger, "msg_1").await;
    assert_eq!(d.status, DeliveryStatus::Failed);
    assert_eq!(d.last_http_status, Some(404));
    assert_eq!(sender.call_count(), 1, "a 404 is never retried");
}

// ---------- egress validation: a blocked endpoint never leaves the process ----------

#[tokio::test]
async fn a_blocked_egress_target_fails_without_a_post() {
    let ledger = InMemoryLedger::new();
    let mut new = enqueued("msg_1");
    new.endpoint = "https://169.254.169.254/latest/meta-data/".to_string();
    ledger.enqueue(new).await.unwrap();
    let sender = ScriptedSender::new([]);
    let engine = DeliveryEngine::new(ledger.clone(), sender.clone(), secrets(), config(5));

    let report = engine.tick(at(UNIX_EPOCH, 1_000)).await.unwrap();
    assert_eq!(report.failed, 1);
    let d = row(&ledger, "msg_1").await;
    assert_eq!(d.status, DeliveryStatus::Failed);
    assert_eq!(
        sender.call_count(),
        0,
        "a blocked target is never POSTed to"
    );
}

// ---------- unknown subscription (no signer) ----------

#[tokio::test]
async fn an_unknown_subscription_fails_permanently() {
    let ledger = InMemoryLedger::new();
    let mut new = enqueued("msg_1");
    new.subscription = "sub_unknown".to_string();
    ledger.enqueue(new).await.unwrap();
    let sender = ScriptedSender::new([]);
    let engine = DeliveryEngine::new(ledger.clone(), sender.clone(), secrets(), config(5));

    let report = engine.tick(at(UNIX_EPOCH, 1_000)).await.unwrap();
    assert_eq!(report.failed, 1);
    assert_eq!(sender.call_count(), 0, "no signer ⇒ no POST");
    assert_eq!(row(&ledger, "msg_1").await.status, DeliveryStatus::Failed);
}

// ---------- the per-POST timeout is a transient failure ----------

#[tokio::test(start_paused = true)]
async fn a_slow_receiver_times_out_and_is_retried() {
    let ledger = InMemoryLedger::new();
    ledger.enqueue(enqueued("msg_1")).await.unwrap();
    // First POST hangs past the 100 ms timeout; the retry then succeeds.
    let sender = ScriptedSender::new([Reply::Hang, Reply::Respond(200, "ok".to_string())]);
    let mut cfg = config(5);
    cfg.breaker.failure_threshold = 100;
    let engine = DeliveryEngine::new(ledger.clone(), sender.clone(), secrets(), cfg);

    let t0 = at(UNIX_EPOCH, 1_000);
    let r1 = engine.tick(t0).await.unwrap();
    assert_eq!((r1.retried, r1.delivered), (1, 0), "a timeout is transient");
    let d = row(&ledger, "msg_1").await;
    assert_eq!(d.status, DeliveryStatus::Pending);
    assert_eq!(d.attempt, 1);

    // The retry, once due, succeeds.
    let r2 = engine.tick(d.next_attempt_at).await.unwrap();
    assert_eq!(r2.delivered, 1);
    assert_eq!(
        row(&ledger, "msg_1").await.status,
        DeliveryStatus::Delivered
    );
}

// ---------- per-endpoint circuit breaker: a dead endpoint is deferred ----------

#[tokio::test]
async fn a_dead_endpoints_breaker_opens_and_its_rows_are_deferred() {
    let ledger = InMemoryLedger::new();
    // Two deliveries to the SAME endpoint.
    ledger.enqueue(enqueued("msg_1")).await.unwrap();
    let mut second = enqueued("msg_2");
    second.id = DeliveryId::new("msg_2");
    ledger.enqueue(second).await.unwrap();

    // Every POST is a transport failure; the breaker trips after one (config).
    let sender = ScriptedSender::new([
        Reply::Transport("connection refused".to_string()),
        Reply::Transport("connection refused".to_string()),
    ]);
    let engine = DeliveryEngine::new(ledger.clone(), sender.clone(), secrets(), config(5));

    let t0 = at(UNIX_EPOCH, 1_000);
    let report = engine.tick(t0).await.unwrap();
    assert_eq!(report.claimed, 2);
    // The first POST fails transiently and trips the breaker; the second row,
    // same endpoint, is deferred without a POST.
    assert_eq!(report.retried, 1, "the first attempt is a transient retry");
    assert_eq!(
        report.deferred, 1,
        "the second row is deferred by the open breaker"
    );
    assert_eq!(
        sender.call_count(),
        1,
        "the open breaker spares the dead endpoint a second POST"
    );

    // The deferred row kept its attempt budget (the endpoint was down, not the
    // payload's fault) and is rescheduled past the breaker cooldown.
    let deferred = [row(&ledger, "msg_1").await, row(&ledger, "msg_2").await]
        .into_iter()
        .find(|d| d.last_response_excerpt.as_deref() == Some("endpoint circuit open; deferred"))
        .expect("one row was deferred");
    assert_eq!(deferred.status, DeliveryStatus::Pending);
    assert_eq!(deferred.attempt, 0, "a deferral spends no attempt");
    assert_eq!(deferred.next_attempt_at, t0 + Duration::from_secs(30));
}
