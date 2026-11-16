//! `seiva-client` — the inter-service client: how one Seiva module calls
//! **another module's service** (its facade over HTTP/gRPC) once the two are
//! split into separate microservices, with the framework's resilience guardrails
//! wrapped around every call.
//!
//! This is the first real consumer of `seiva-resilience`. A call is composed
//! from the inside out, exactly as that crate prescribes — time each attempt,
//! retry the transient failures on the framework's one schedule, and guard the
//! whole thing with a breaker so a dependency that stays down stops being called:
//!
//! ```text
//! breaker.call(                     // shed load if the dependency is already down
//!     retry.retry(                  // absorb a transient failure (one schedule)
//!         timeout(budget, attempt)  // cut a slow attempt off
//!     )
//! )
//! ```
//!
//! # The breaker counts calls, not attempts
//!
//! The retry lives **inside** the breaker, so the breaker records exactly one
//! outcome per [`ServiceClient::call`], not one per retried attempt. A transient
//! failure the retry absorbs is invisible to the breaker — a flaky dependency
//! that recovers within the retry budget never trips it. A dependency that is
//! genuinely down (every attempt fails) surfaces as a single breaker failure;
//! after [`failure_threshold`](seiva_resilience::BreakerConfig::failure_threshold)
//! such calls the breaker opens and sheds load. This is the whole point of the
//! composition order, and it is what the crate's tests prove.
//!
//! # Transport-agnostic
//!
//! The crate names no HTTP or gRPC client. It composes futures around a
//! [`Transport`] seam the caller injects — the same adapter pattern as the rest
//! of the framework — so it builds and tests fully offline against an in-memory
//! transport. The concrete HTTP/gRPC transport is a downstream adapter, not a
//! dependency of this crate.
//!
//! # Data-layer-free
//!
//! Like `seiva-resilience`, this crate only composes futures; it never touches a
//! database, a broker or the DORM gateway, so it lives in the default workspace
//! members and carries no `dorm` feature.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::time::Duration;

use async_trait::async_trait;
use seiva_resilience::{timeout, BreakerError, CircuitBreaker, RetryPolicy, TimedOut};

pub use seiva_resilience::{BreakerConfig, BreakerState};

/// A request to another module's service, independent of the wire transport.
///
/// The caller fills these fields from whatever contract it is honouring (a
/// facade route, a gRPC method) and the injected [`Transport`] turns them into
/// an actual request. Kept to owned, cloneable fields because the retry re-runs
/// the operation: each attempt gets a fresh clone of the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRequest {
    /// The request method (e.g. `"POST"`), as the transport understands it.
    pub method: String,
    /// The target path on the remote service (e.g. `"/v1/notes"`).
    pub path: String,
    /// Header name/value pairs to send, in order.
    pub headers: Vec<(String, String)>,
    /// The raw request body, byte for byte.
    pub body: Vec<u8>,
}

impl ServiceRequest {
    /// A request with no headers and an empty body — the common starting point a
    /// caller then fills in.
    #[must_use]
    pub fn new(method: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
}

/// A response from another module's service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceResponse {
    /// The HTTP-style status code the dependency answered with.
    pub status: u16,
    /// Response header name/value pairs, in the order received.
    pub headers: Vec<(String, String)>,
    /// The raw response body, byte for byte.
    pub body: Vec<u8>,
}

impl ServiceResponse {
    /// A bare response carrying just a status, for adapters/tests that do not set
    /// headers or a body.
    #[must_use]
    pub fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Whether the status is a 2xx success.
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Whether the status is a 5xx server error — the dependency answered but is
    /// unhealthy, which [`ServiceClient::call`] treats as a transient failure
    /// (retried, and counted against the breaker when the budget is spent).
    #[must_use]
    pub fn is_server_error(&self) -> bool {
        self.status >= 500
    }
}

/// A transport-level failure: the dependency gave **no HTTP answer** at all — a
/// refused connection, a reset, a DNS or TLS failure. Distinct from a 5xx, which
/// is an answer (a [`ServiceResponse`]). Both are transient, so both are retried.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("transport failure: {0}")]
pub struct TransportError(pub String);

impl TransportError {
    /// Build a transport error from any message.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The seam between the resilience-wrapping client and the actual wire: perform
/// **one** request/response round-trip.
///
/// A [`ServiceClient`] calls this once per attempt; the breaker, retry and
/// timeout around it are the client's job, so an implementation does only the
/// raw send and either returns the [`ServiceResponse`] (any status, including a
/// 5xx) or a [`TransportError`] when no answer came back. It must not retry or
/// time out internally — that would double the client's guardrails.
///
/// Object-safe: a caller can store the client over `Arc<dyn Transport>` or a
/// concrete adapter. The concrete HTTP/gRPC transport is a downstream adapter.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send `request` once and return the response, or a [`TransportError`] if the
    /// dependency gave no answer.
    async fn roundtrip(&self, request: ServiceRequest) -> Result<ServiceResponse, TransportError>;
}

/// Everything a [`ServiceClient::call`] can fail with.
///
/// A 2xx/3xx/4xx response is **not** an error here: it is an answer the
/// dependency is healthy enough to give, returned as `Ok(ServiceResponse)` for
/// the caller to interpret (a 4xx is the caller's own bad request, not a sign the
/// dependency is down). Only the variants below — all transient — mean the call
/// did not get a usable answer.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The breaker was open (or half-open at its probe limit): the call was
    /// **not run**. Fail fast or fall back; do not retry into an open breaker.
    #[error("circuit breaker is open: call rejected")]
    Open,
    /// Every attempt outlived its per-attempt timeout budget.
    #[error("request timed out after {0:?} per attempt")]
    Timeout(Duration),
    /// The transport gave no answer on every attempt (connection refused, reset…).
    #[error("transport failure: {0}")]
    Transport(String),
    /// The dependency answered 5xx on every attempt — up, but unhealthy.
    #[error("dependency returned server error {0}")]
    ServerError(u16),
}

impl ClientError {
    /// Whether this failure is worth retrying. Every current variant that a retry
    /// attempt can produce is transient; [`ClientError::Open`] never reaches the
    /// retry (it is the breaker rejecting the whole call), so it is excluded.
    fn is_transient(&self) -> bool {
        matches!(
            self,
            ClientError::Timeout(_) | ClientError::Transport(_) | ClientError::ServerError(_)
        )
    }
}

/// A resilient client to one remote dependency: it wraps every call in a circuit
/// breaker, a retry policy and a per-attempt timeout, over an injected
/// [`Transport`].
///
/// Clone it freely — the breaker is shared across clones (an `Arc` inner), so one
/// logical dependency has one breaker however many handles call it.
#[derive(Clone)]
pub struct ServiceClient<T> {
    transport: T,
    breaker: CircuitBreaker,
    retry: RetryPolicy,
    attempt_timeout: Duration,
}

impl<T: Transport> ServiceClient<T> {
    /// Build a client from its transport and its three guardrails.
    ///
    /// `attempt_timeout` bounds **each** attempt (not the whole retried call), so
    /// the retry can move on from a stuck attempt instead of waiting out a dead
    /// connection; pass [`Duration::MAX`] to disable it.
    #[must_use]
    pub fn new(
        transport: T,
        breaker: CircuitBreaker,
        retry: RetryPolicy,
        attempt_timeout: Duration,
    ) -> Self {
        Self {
            transport,
            breaker,
            retry,
            attempt_timeout,
        }
    }

    /// The breaker guarding this dependency, for health reporting and metrics.
    #[must_use]
    pub fn breaker(&self) -> &CircuitBreaker {
        &self.breaker
    }

    /// Call the remote dependency, with the breaker, retry and per-attempt timeout
    /// applied.
    ///
    /// Composed from the inside out: each attempt is timed out; a transient
    /// failure (a timeout, a transport error, a 5xx) is retried on the
    /// [`RetryPolicy`] schedule; and the whole retried call runs under the
    /// breaker, so it records one outcome — not one per attempt.
    ///
    /// # Errors
    ///
    /// - [`ClientError::Open`] if the breaker rejected the call without running it.
    /// - [`ClientError::Timeout`] / [`ClientError::Transport`] /
    ///   [`ClientError::ServerError`] if every attempt failed transiently and the
    ///   retry budget was spent; the last failure is reported.
    ///
    /// A non-5xx response (2xx/3xx/4xx) is returned as `Ok` for the caller to
    /// interpret.
    pub async fn call(&self, request: ServiceRequest) -> Result<ServiceResponse, ClientError> {
        let outcome = self
            .breaker
            .call(async {
                self.retry
                    .retry_if(
                        || {
                            let transport = &self.transport;
                            let request = request.clone();
                            let budget = self.attempt_timeout;
                            async move {
                                match timeout(budget, transport.roundtrip(request)).await {
                                    // The attempt outlived its budget.
                                    Err(TimedOut { budget }) => Err(ClientError::Timeout(budget)),
                                    // No answer came back.
                                    Ok(Err(TransportError(message))) => {
                                        Err(ClientError::Transport(message))
                                    }
                                    // An answer, but an unhealthy one: transient.
                                    Ok(Ok(response)) if response.is_server_error() => {
                                        Err(ClientError::ServerError(response.status))
                                    }
                                    // A usable answer (2xx/3xx/4xx) — the caller's to read.
                                    Ok(Ok(response)) => Ok(response),
                                }
                            }
                        },
                        ClientError::is_transient,
                    )
                    .await
            })
            .await;

        match outcome {
            Ok(response) => Ok(response),
            Err(BreakerError::Open) => {
                tracing::warn!("inter-service call rejected: circuit breaker is open");
                Err(ClientError::Open)
            }
            Err(BreakerError::Inner(error)) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};

    /// A transport that replays a scripted sequence of outcomes, then falls back
    /// to a default once the script is exhausted — enough to drive every
    /// retry/breaker path without a real socket.
    struct ScriptedTransport {
        script: Mutex<VecDeque<Result<u16, TransportError>>>,
        default: Result<u16, TransportError>,
        calls: AtomicU32,
    }

    impl ScriptedTransport {
        /// A sequence of outcomes (a status code, or a transport error), then 200s.
        fn sequence(seq: impl IntoIterator<Item = Result<u16, TransportError>>) -> Arc<Self> {
            Arc::new(Self {
                script: Mutex::new(seq.into_iter().collect()),
                default: Ok(200),
                calls: AtomicU32::new(0),
            })
        }

        /// Always the same outcome, for a dependency that stays down.
        fn always(outcome: Result<u16, TransportError>) -> Arc<Self> {
            Arc::new(Self {
                script: Mutex::new(VecDeque::new()),
                default: outcome,
                calls: AtomicU32::new(0),
            })
        }

        fn calls(&self) -> u32 {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Transport for Arc<ScriptedTransport> {
        async fn roundtrip(
            &self,
            _request: ServiceRequest,
        ) -> Result<ServiceResponse, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let next = self
                .script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| self.default.clone());
            next.map(ServiceResponse::status)
        }
    }

    /// A transport that always sleeps past the client's per-attempt budget, so
    /// every attempt times out.
    struct SlowTransport {
        delay: Duration,
        calls: AtomicU32,
    }

    #[async_trait]
    impl Transport for Arc<SlowTransport> {
        async fn roundtrip(
            &self,
            _request: ServiceRequest,
        ) -> Result<ServiceResponse, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            Ok(ServiceResponse::status(200))
        }
    }

    fn req() -> ServiceRequest {
        ServiceRequest::new("POST", "/v1/notes")
    }

    // A zero-base retry so the attempts do not actually sleep between tries.
    fn instant_retry(max_attempts: u32) -> RetryPolicy {
        RetryPolicy {
            max_attempts,
            backoff_base_ms: 0,
        }
    }

    #[tokio::test]
    async fn a_transient_failure_is_absorbed_by_retry_without_tripping_the_breaker() {
        // Two transient failures — a transport reset, then a 503 — and then a 200.
        let transport = ScriptedTransport::sequence([
            Err(TransportError::new("connection reset")),
            Ok(503),
            Ok(200),
        ]);
        // A STRICT breaker: a single consecutive failure would trip it. The point
        // is the breaker never sees the two transient failures — the retry absorbs
        // them, so `call` records exactly one SUCCESS.
        let breaker = CircuitBreaker::new(BreakerConfig {
            failure_threshold: 1,
            ..Default::default()
        });
        let client = ServiceClient::new(
            transport.clone(),
            breaker.clone(),
            instant_retry(5),
            Duration::from_secs(1),
        );

        let response = client
            .call(req())
            .await
            .expect("the retry should absorb the two transient failures");
        assert_eq!(response.status, 200);
        assert_eq!(transport.calls(), 3, "it retried twice, then succeeded");
        assert_eq!(
            breaker.state(),
            BreakerState::Closed,
            "a breaker with threshold 1 stays closed: it saw one success, not three failures"
        );
    }

    #[tokio::test]
    async fn retries_do_not_inflate_the_breaker_failure_count() {
        // A dependency that is down: every attempt answers 503. With a 3-attempt
        // budget, one `call` burns three attempts but must count as ONE breaker
        // failure — not three — so a transient blip can never trip a breaker whose
        // threshold is above one.
        let transport = ScriptedTransport::always(Ok(503));
        let breaker = CircuitBreaker::new(BreakerConfig {
            failure_threshold: 2,
            open_cooldown: Duration::from_secs(30),
            ..Default::default()
        });
        let client = ServiceClient::new(
            transport.clone(),
            breaker.clone(),
            instant_retry(3),
            Duration::from_secs(1),
        );

        let first = client.call(req()).await;
        assert!(matches!(first, Err(ClientError::ServerError(503))));
        assert_eq!(transport.calls(), 3, "the whole retry budget is one call");
        assert_eq!(
            breaker.state(),
            BreakerState::Closed,
            "one exhausted call is ONE breaker failure, not three"
        );

        let second = client.call(req()).await;
        assert!(matches!(second, Err(ClientError::ServerError(503))));
        assert_eq!(
            breaker.state(),
            BreakerState::Open,
            "the second failed call reaches the threshold and trips the breaker"
        );

        // Now open: the next call must be rejected WITHOUT running the transport.
        let before = transport.calls();
        let rejected = client.call(req()).await;
        assert!(matches!(rejected, Err(ClientError::Open)));
        assert_eq!(
            transport.calls(),
            before,
            "an open breaker rejects without running the operation"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn attempts_that_outlive_the_budget_time_out_and_are_retried() {
        // Each attempt sleeps ten seconds; the per-attempt budget is 50 ms, so
        // every attempt times out and is retried. The three timed-out attempts are
        // ONE breaker failure (threshold 5, so still closed).
        let transport = Arc::new(SlowTransport {
            delay: Duration::from_secs(10),
            calls: AtomicU32::new(0),
        });
        let breaker = CircuitBreaker::new(BreakerConfig {
            failure_threshold: 5,
            ..Default::default()
        });
        let client = ServiceClient::new(
            transport.clone(),
            breaker.clone(),
            instant_retry(3),
            Duration::from_millis(50),
        );

        let result = client.call(req()).await;
        assert!(matches!(result, Err(ClientError::Timeout(_))));
        assert_eq!(
            transport.calls.load(Ordering::SeqCst),
            3,
            "each attempt timed out and was retried up to the budget"
        );
        assert_eq!(
            breaker.state(),
            BreakerState::Closed,
            "three timed-out attempts are one breaker failure; threshold five is not reached"
        );
    }

    #[tokio::test]
    async fn a_client_error_response_is_returned_not_retried() {
        // A 404 is a usable answer: the dependency is healthy, the caller sent a
        // bad request. It is returned as Ok for the caller to read, NOT retried,
        // and does not count against the breaker.
        let transport = ScriptedTransport::always(Ok(404));
        let breaker = CircuitBreaker::new(BreakerConfig {
            failure_threshold: 1,
            ..Default::default()
        });
        let client = ServiceClient::new(
            transport.clone(),
            breaker.clone(),
            instant_retry(5),
            Duration::from_secs(1),
        );

        let response = client.call(req()).await.expect("a 4xx is a usable answer");
        assert_eq!(response.status, 404);
        assert!(!response.is_success());
        assert_eq!(transport.calls(), 1, "a 4xx is not retried");
        assert_eq!(
            breaker.state(),
            BreakerState::Closed,
            "a 4xx does not count against the breaker"
        );
    }
}
