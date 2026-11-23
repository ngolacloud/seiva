//! The [`DeliveryEngine`]: assemble the primitives into one reliable send.
//!
//! A worker host calls [`tick`](DeliveryEngine::tick) on a cadence. Each tick
//! claims a batch of due deliveries under a lease and, for each, decides an
//! outcome and records it:
//!
//! 1. Resolve the subscription's signer ([`SigningSecrets`]) — unknown ⇒ a
//!    permanent [`Failed`](super::DeliveryStatus::Failed).
//! 2. [Validate egress](super::validate_egress) — blocked ⇒ permanent `Failed`.
//! 3. Sign with [`Webhook`], then POST through the destination's
//!    [`CircuitBreaker`], each POST bounded by a
//!    [`timeout`](seiva_resilience::timeout). The breaker counts only
//!    *transport-level* failures (a timeout, a connection error) as endpoint
//!    unhealth, so a receiver that answers `4xx` stays healthy for everyone else.
//! 4. Map the result: a `2xx` ⇒ [`Delivered`](super::DeliveryStatus::Delivered); a
//!    retryable status / timeout / transport error ⇒ reschedule with backoff, or
//!    [`Dead`](super::DeliveryStatus::Dead) once the attempt cap is spent; a
//!    non-retryable `4xx` ⇒ permanent `Failed`; an **open breaker** ⇒ *deferred*
//!    without spending an attempt.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use seiva_resilience::{BreakerConfig, BreakerError, CircuitBreaker, RetryPolicy};

use super::ledger::{Delivery, DeliveryLedger, DeliveryStatus, DeliveryUpdate, LedgerError};
use super::validate_egress;
use crate::Webhook;

/// The request an [`HttpSender`] POSTs: the destination plus the three Standard
/// Webhooks headers already computed. Borrowed from the [`Delivery`] and the
/// freshly-signed signature, so building it allocates nothing.
#[derive(Debug, Clone)]
pub struct HttpRequest<'a> {
    /// The destination URL (already egress-validated by the engine).
    pub url: &'a str,
    /// The value for the [`webhook-id`](crate::ID_HEADER) header (= the
    /// idempotency key).
    pub id: &'a str,
    /// The Unix-seconds send time for the [`webhook-timestamp`](crate::TIMESTAMP_HEADER)
    /// header; the same value the signature was computed over.
    pub timestamp: i64,
    /// The `v1,…` value for the [`webhook-signature`](crate::SIGNATURE_HEADER)
    /// header.
    pub signature: &'a str,
    /// The `Content-Type` to send the body as.
    pub content_type: &'a str,
    /// The raw request body.
    pub body: &'a [u8],
}

/// A receiver's response, reduced to what the engine decides on: the HTTP status
/// and a short body excerpt for the ledger.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// The HTTP status code.
    pub status: u16,
    /// A short excerpt of the response body (for debugging; the engine truncates
    /// it to the configured limit before recording).
    pub body_excerpt: String,
}

impl HttpResponse {
    /// Build a response.
    #[must_use]
    pub fn new(status: u16, body_excerpt: impl Into<String>) -> Self {
        Self {
            status,
            body_excerpt: body_excerpt.into(),
        }
    }

    /// Whether the status is a `2xx` success.
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// A transport-level failure from an [`HttpSender`]: no HTTP response came back
/// (a connection refused/reset, a DNS error, a TLS failure). It is treated as a
/// **transient** failure — retried, and counted against the endpoint's breaker.
/// The message is safe to log: it carries no payload or secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("webhook transport error: {0}")]
pub struct SendError(String);

impl SendError {
    /// Build a transport error from a message.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The one seam that touches the network, kept a port so this crate links no HTTP
/// client. A product binds a `reqwest`/`hyper` adapter that sets the three
/// Standard Webhooks headers from the [`HttpRequest`]; a test binds a scripted
/// double.
#[async_trait]
pub trait HttpSender: Send + Sync {
    /// POST `request` to its URL with the Standard Webhooks headers, returning the
    /// receiver's [`HttpResponse`] — or a [`SendError`] if no response was
    /// obtained at all (a transport failure, distinct from an HTTP error status).
    async fn post(&self, request: HttpRequest<'_>) -> Result<HttpResponse, SendError>;
}

/// Resolves the [`Webhook`] signer for a subscription. The secret
/// lives here — **never in the ledger** — so the durable record carries no key
/// material. Kept synchronous: a signer lookup is an in-memory/cache hit, not a
/// round trip.
pub trait SigningSecrets: Send + Sync {
    /// The signer for `subscription`, or `None` if the subscription is unknown —
    /// in which case the delivery is permanently [`Failed`](super::DeliveryStatus::Failed)
    /// (a misconfiguration, not something a retry fixes).
    fn signer(&self, subscription: &str) -> Option<Webhook>;
}

/// How the engine retries, times out and trips.
#[derive(Debug, Clone)]
pub struct DeliveryConfig {
    /// The retry schedule: its `backoff` paces the delay before the next attempt
    /// (the framework-wide exponential schedule, shared with the outbox relay),
    /// and its `max_attempts` is the **delivery attempt cap** — past it a
    /// transiently-failing row is dead-lettered.
    pub retry: RetryPolicy,
    /// The per-POST timeout budget. A receiver slower than this is a transient
    /// failure, cancelled and retried.
    pub post_timeout: Duration,
    /// The per-endpoint circuit breaker configuration.
    pub breaker: BreakerConfig,
    /// How long a claimed row is leased to one worker before another may reclaim
    /// it (crash recovery).
    pub lease: Duration,
    /// How many due rows one [`tick`](DeliveryEngine::tick) claims.
    pub batch: usize,
    /// The `Content-Type` of the POSTed body.
    pub content_type: String,
    /// The maximum length (in bytes) of a recorded response excerpt.
    pub excerpt_limit: usize,
}

impl Default for DeliveryConfig {
    fn default() -> Self {
        Self {
            retry: RetryPolicy::default(),
            post_timeout: Duration::from_secs(10),
            breaker: BreakerConfig::default(),
            lease: Duration::from_secs(30),
            batch: 32,
            content_type: "application/json".to_string(),
            excerpt_limit: 512,
        }
    }
}

/// What one [`tick`](DeliveryEngine::tick) did — a tally per outcome, for metrics
/// and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickReport {
    /// How many due rows were claimed this tick.
    pub claimed: usize,
    /// Delivered successfully (a `2xx`).
    pub delivered: usize,
    /// Transiently failed and rescheduled for a later attempt.
    pub retried: usize,
    /// Transiently failed with the attempt cap spent — dead-lettered.
    pub dead: usize,
    /// Permanently failed (blocked egress, unknown signer, non-retryable status).
    pub failed: usize,
    /// Deferred because the endpoint's breaker was open (no attempt spent).
    pub deferred: usize,
    /// The outcome was decided but the ledger write failed; the row stays
    /// `in_flight` for lease recovery. Counted so a tick never silently drops one.
    pub record_errors: usize,
}

/// Everything the engine surfaces to its caller.
#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    /// The ledger could not be queried for due work.
    #[error(transparent)]
    Ledger(#[from] LedgerError),
}

/// The decided outcome of one delivery, before it is tallied.
enum Outcome {
    Delivered,
    Retried,
    Dead,
    Failed,
    Deferred,
    RecordError,
}

/// A transport-level attempt failure, as seen inside the breaker-guarded call.
/// Private: it never leaves the engine.
enum AttemptFailure {
    Timeout,
    Send(SendError),
}

/// The durable, retrying outbound-delivery engine (ADR 0033).
///
/// Generic over its three ports so a test binds in-memory doubles and a product
/// binds real adapters, with no `dyn` indirection. Construct with [`new`](Self::new)
/// and drive with [`tick`](Self::tick).
pub struct DeliveryEngine<L, S, K> {
    ledger: L,
    sender: S,
    secrets: K,
    config: DeliveryConfig,
    /// One breaker per destination endpoint, created on first use. Keyed by the
    /// endpoint URL so one dead receiver trips only its own circuit.
    breakers: Mutex<HashMap<String, CircuitBreaker>>,
}

impl<L, S, K> DeliveryEngine<L, S, K>
where
    L: DeliveryLedger,
    S: HttpSender,
    K: SigningSecrets,
{
    /// Assemble an engine from its ports and configuration.
    pub fn new(ledger: L, sender: S, secrets: K, config: DeliveryConfig) -> Self {
        Self {
            ledger,
            sender,
            secrets,
            config,
            breakers: Mutex::new(HashMap::new()),
        }
    }

    /// The ledger this engine drives, for the caller to enqueue and inspect.
    pub fn ledger(&self) -> &L {
        &self.ledger
    }

    /// Claim and deliver one batch of due rows, recording each outcome.
    ///
    /// `now` is injected (like [`Webhook::verify`](crate::Webhook::verify)) so the
    /// due-window, the backoff schedule and the signature timestamp are all
    /// deterministic under test; pass `SystemTime::now()` in production.
    ///
    /// # Errors
    ///
    /// Returns [`DeliveryError::Ledger`] only if the *claim* fails; a per-delivery
    /// record failure is tallied in [`TickReport::record_errors`], never aborting
    /// the rest of the batch.
    pub async fn tick(&self, now: SystemTime) -> Result<TickReport, DeliveryError> {
        let due = self
            .ledger
            .claim_due(now, self.config.batch, self.config.lease)
            .await?;

        let mut report = TickReport {
            claimed: due.len(),
            ..TickReport::default()
        };
        for delivery in &due {
            match self.deliver_one(delivery, now).await {
                Outcome::Delivered => report.delivered += 1,
                Outcome::Retried => report.retried += 1,
                Outcome::Dead => report.dead += 1,
                Outcome::Failed => report.failed += 1,
                Outcome::Deferred => report.deferred += 1,
                Outcome::RecordError => report.record_errors += 1,
            }
        }
        Ok(report)
    }

    /// Decide and persist the outcome of one delivery.
    async fn deliver_one(&self, delivery: &Delivery, now: SystemTime) -> Outcome {
        let (update, outcome) = self.decide(delivery, now).await;
        match self.ledger.record(&delivery.id, update).await {
            Ok(()) => outcome,
            Err(error) => {
                tracing::error!(
                    delivery = %delivery.id,
                    %error,
                    "failed to record delivery outcome; row stays in_flight for lease recovery"
                );
                Outcome::RecordError
            }
        }
    }

    /// Resolve → validate → sign → POST, returning the ledger mutation and the
    /// tally outcome without touching the ledger (so [`deliver_one`](Self::deliver_one)
    /// owns the single `record`).
    async fn decide(&self, delivery: &Delivery, now: SystemTime) -> (DeliveryUpdate, Outcome) {
        // 1. Signer. An unknown subscription is a permanent misconfiguration.
        let Some(signer) = self.secrets.signer(&delivery.subscription) else {
            tracing::warn!(
                delivery = %delivery.id,
                subscription = %delivery.subscription,
                "no signer for subscription; delivery permanently failed"
            );
            return (
                self.terminal(
                    DeliveryStatus::Failed,
                    delivery.attempt,
                    now,
                    None,
                    Some("no signing secret for subscription".to_string()),
                ),
                Outcome::Failed,
            );
        };

        // 2. Egress. A blocked destination is a permanent failure (SSRF guard).
        if let Err(error) = validate_egress(&delivery.endpoint) {
            tracing::warn!(delivery = %delivery.id, %error, "egress blocked; delivery permanently failed");
            return (
                self.terminal(
                    DeliveryStatus::Failed,
                    delivery.attempt,
                    now,
                    None,
                    Some(error.to_string()),
                ),
                Outcome::Failed,
            );
        }

        // 3. Sign and POST under the endpoint's breaker and a per-POST timeout.
        let timestamp = crate::unix_secs(now);
        let signature = signer.sign(delivery.id.as_str(), timestamp, &delivery.payload);
        let breaker = self.breaker_for(&delivery.endpoint);
        let request = HttpRequest {
            url: &delivery.endpoint,
            id: delivery.id.as_str(),
            timestamp,
            signature: signature.as_str(),
            content_type: &self.config.content_type,
            body: &delivery.payload,
        };
        let post_timeout = self.config.post_timeout;
        let sender = &self.sender;
        let outcome = breaker
            .call(async move {
                match seiva_resilience::timeout(post_timeout, sender.post(request)).await {
                    // The endpoint responded (any status): a breaker *success*,
                    // whatever the HTTP code — it is up.
                    Ok(Ok(response)) => Ok(response),
                    // A transport error or a timeout: the endpoint is unhealthy,
                    // so it counts against the breaker.
                    Ok(Err(send_error)) => Err(AttemptFailure::Send(send_error)),
                    Err(_timed_out) => Err(AttemptFailure::Timeout),
                }
            })
            .await;

        // The attempt number just made (attempts already made + this one).
        let n = delivery.attempt + 1;
        match outcome {
            Ok(response) if response.is_success() => {
                tracing::debug!(delivery = %delivery.id, status = response.status, "delivered");
                let excerpt = self.excerpt(&response.body_excerpt);
                (
                    self.terminal(
                        DeliveryStatus::Delivered,
                        n,
                        now,
                        Some(response.status),
                        Some(excerpt),
                    ),
                    Outcome::Delivered,
                )
            }
            Ok(response) if is_retryable_status(response.status) => {
                let excerpt = self.excerpt(&response.body_excerpt);
                self.schedule_or_dead(delivery, n, now, Some(response.status), Some(excerpt))
            }
            Ok(response) => {
                // A client error the receiver will not change its mind on.
                tracing::warn!(delivery = %delivery.id, status = response.status, "permanent HTTP rejection");
                let excerpt = self.excerpt(&response.body_excerpt);
                (
                    self.terminal(
                        DeliveryStatus::Failed,
                        n,
                        now,
                        Some(response.status),
                        Some(excerpt),
                    ),
                    Outcome::Failed,
                )
            }
            Err(BreakerError::Open) => {
                // The endpoint's circuit is open: defer this row WITHOUT spending
                // an attempt, so a dead receiver cannot burn a delivery's budget.
                tracing::debug!(delivery = %delivery.id, endpoint = %delivery.endpoint, "endpoint circuit open; deferring");
                let defer_until = now + self.config.breaker.open_cooldown;
                (
                    DeliveryUpdate {
                        status: DeliveryStatus::Pending,
                        attempt: delivery.attempt,
                        next_attempt_at: defer_until,
                        last_http_status: None,
                        last_response_excerpt: Some("endpoint circuit open; deferred".to_string()),
                    },
                    Outcome::Deferred,
                )
            }
            Err(BreakerError::Inner(failure)) => {
                let reason = match failure {
                    AttemptFailure::Timeout => "POST timed out".to_string(),
                    AttemptFailure::Send(error) => error.to_string(),
                };
                tracing::warn!(delivery = %delivery.id, reason = %reason, "transient delivery failure");
                self.schedule_or_dead(delivery, n, now, None, Some(reason))
            }
        }
    }

    /// A transient failure: reschedule the next attempt with backoff, or
    /// dead-letter the row once the attempt cap is spent.
    fn schedule_or_dead(
        &self,
        delivery: &Delivery,
        attempt_made: u32,
        now: SystemTime,
        http_status: Option<u16>,
        excerpt: Option<String>,
    ) -> (DeliveryUpdate, Outcome) {
        let cap = self.config.retry.max_attempts.max(1);
        if attempt_made >= cap {
            tracing::warn!(
                delivery = %delivery.id,
                attempts = attempt_made,
                "delivery attempt cap reached; dead-lettered"
            );
            (
                self.terminal(
                    DeliveryStatus::Dead,
                    attempt_made,
                    now,
                    http_status,
                    excerpt,
                ),
                Outcome::Dead,
            )
        } else {
            let backoff = self.config.retry.backoff(attempt_made);
            (
                DeliveryUpdate {
                    status: DeliveryStatus::Pending,
                    attempt: attempt_made,
                    next_attempt_at: now + backoff,
                    last_http_status: http_status,
                    last_response_excerpt: excerpt,
                },
                Outcome::Retried,
            )
        }
    }

    /// A terminal update (`next_attempt_at` is irrelevant and set to `now`).
    fn terminal(
        &self,
        status: DeliveryStatus,
        attempt: u32,
        now: SystemTime,
        http_status: Option<u16>,
        excerpt: Option<String>,
    ) -> DeliveryUpdate {
        DeliveryUpdate {
            status,
            attempt,
            next_attempt_at: now,
            last_http_status: http_status,
            last_response_excerpt: excerpt,
        }
    }

    /// The breaker guarding `endpoint`, created on first use. Every clone shares
    /// state, so one dead endpoint trips its own circuit across all workers.
    fn breaker_for(&self, endpoint: &str) -> CircuitBreaker {
        let mut map = self.breakers.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(endpoint.to_string())
            .or_insert_with(|| CircuitBreaker::new(self.config.breaker))
            .clone()
    }

    /// Truncate a response body to the configured excerpt limit, on a char
    /// boundary, marking a cut with an ellipsis.
    fn excerpt(&self, body: &str) -> String {
        let limit = self.config.excerpt_limit;
        if body.len() <= limit {
            return body.to_string();
        }
        let mut end = limit;
        while end > 0 && !body.is_char_boundary(end) {
            end -= 1;
        }
        let mut truncated = body[..end].to_string();
        truncated.push('…');
        truncated
    }
}

/// Whether an HTTP status is worth retrying: request-timeout, too-early,
/// too-many-requests, and the whole `5xx` server-error class. Any other `4xx` is
/// a permanent rejection the receiver will not reconsider.
fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429) || (500..600).contains(&status)
}
