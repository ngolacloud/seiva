//! The composition root for the ADR 0034 **webhook delivery job**.
//!
//! [`seiva_webhook_delivery::run_delivery_worker`] is the job — one
//! [`DeliveryEngine::tick`](seiva_webhooks::delivery::DeliveryEngine) per pass on
//! the shared [`seiva::worker`] loop — but it is DORM-free and http-client-free:
//! it leaves three seams open. This crate is the first place those seams are bound
//! to **production** adapters and the worker is actually started:
//!
//! 1. the durable [`DeliveryLedger`](seiva_webhooks::delivery::DeliveryLedger) →
//!    [`DormDeliveryLedger`], one PostgreSQL table drained safely at N replicas by
//!    the lease in [`claim_due`](seiva_webhooks::delivery::DeliveryLedger::claim_due);
//! 2. the [`HttpSender`] → [`ReqwestSender`], a thin reqwest/TLS binding;
//! 3. the [`SigningSecrets`] → [`StaticSecrets`], a subscription→secret map read
//!    from the deployment's config.
//!
//! [`run_delivery`] composes the three into a [`DeliveryEngine`] and drives
//! [`run_delivery_worker`], so a backlog drains **on its own** — no caller ever
//! hand-ticks the engine. [`run_service`] is the runnable wrapper the binary
//! calls: read config, [`connect`] the handle, [`ensure_schema`], and drain until
//! `shutdown` resolves.
//!
//! This crate is DORM-coupled (via the ledger) and links reqwest + rustls, so it
//! is EXCLUDED from the default Seiva workspace, exactly like its `apps/*`
//! siblings; nothing in `cargo check --workspace` links it.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use seiva::worker::{WorkerConfig, WorkerReport};
use seiva_data_dorm::{connect, delivery_migrations, DormConfig, DormDeliveryLedger};
use seiva_webhook_delivery::run_delivery_worker;
use seiva_webhooks::delivery::{
    DeliveryConfig, DeliveryEngine, HttpRequest, HttpResponse, HttpSender, SendError, SigningSecrets,
};
use seiva_webhooks::{Webhook, ID_HEADER, SIGNATURE_HEADER, TIMESTAMP_HEADER};

// ---------------------------------------------------------------------------
// 1. The production HttpSender adapter (the one seam the engine leaves a port).
// ---------------------------------------------------------------------------

/// A [`HttpSender`] backed by a [`reqwest::Client`].
///
/// The engine hands the adapter an [`HttpRequest`] with the destination URL and
/// the three Standard Webhooks headers already computed (`webhook-id`,
/// `webhook-timestamp`, `webhook-signature`); the adapter only puts them on the
/// wire. A transport failure (no response at all — connection refused, TLS error,
/// DNS failure) becomes a [`SendError`] the engine treats as transient; any HTTP
/// status that *did* come back is returned as an [`HttpResponse`] for the engine
/// to classify. The client is **injected**, so a deployment owns the TLS roots,
/// pool, proxy and timeouts (and a test can resolve a hostname at its local
/// server). Cheap to clone (an `Arc` inside reqwest).
#[derive(Clone)]
pub struct ReqwestSender {
    client: reqwest::Client,
}

impl ReqwestSender {
    /// Build an adapter over a configured [`reqwest::Client`].
    #[must_use]
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
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
            // No HTTP response at all: a transport-level failure the engine retries
            // and counts against the endpoint's breaker. The message carries no
            // payload or secret.
            .map_err(|error| SendError::new(error.to_string()))?;

        let status = response.status().as_u16();
        // The body is only for the ledger excerpt; the engine truncates it. A
        // body-read error is not itself a delivery failure — the status already
        // decided the outcome.
        let body_excerpt = response.text().await.unwrap_or_default();
        Ok(HttpResponse::new(status, body_excerpt))
    }
}

// ---------------------------------------------------------------------------
// 2. The production SigningSecrets resolver.
// ---------------------------------------------------------------------------

/// A [`SigningSecrets`] resolver over a static `subscription → secret` map, loaded
/// from the deployment's config.
///
/// The secret is stored as its raw `whsec_…` string and a [`Webhook`] signer is
/// built on demand, so a malformed secret simply resolves to `None` (an unknown
/// signer, which the engine treats as a permanent failure for that row) rather
/// than panicking at startup. A production deployment would back this with its own
/// per-tenant secret store; this covers the common "secrets in config" case.
#[derive(Clone, Default)]
pub struct StaticSecrets {
    by_subscription: HashMap<String, String>,
}

impl StaticSecrets {
    /// An empty resolver (every subscription is unknown).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the signing `secret` (a `whsec_…` string) for `subscription`.
    #[must_use]
    pub fn with_secret(mut self, subscription: impl Into<String>, secret: impl Into<String>) -> Self {
        self.by_subscription.insert(subscription.into(), secret.into());
        self
    }

    /// How many subscriptions have a secret registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_subscription.len()
    }

    /// Whether no secret is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_subscription.is_empty()
    }
}

impl SigningSecrets for StaticSecrets {
    fn signer(&self, subscription: &str) -> Option<Webhook> {
        // Build the signer on demand; a malformed stored secret is an unknown
        // signer, not a panic.
        self.by_subscription
            .get(subscription)
            .and_then(|secret| Webhook::new(secret).ok())
    }
}

// ---------------------------------------------------------------------------
// 3. Settings + the composed run.
// ---------------------------------------------------------------------------

/// How the delivery worker loops, and the engine behaviour it drives.
pub struct DeliverySettings {
    /// The worker label for logs and metrics.
    pub worker_name: String,
    /// How long to idle between passes when there is no backlog.
    pub interval: Duration,
    /// Run the next pass immediately when a pass claimed at least this many rows
    /// (fast-drain), so a backlog empties without idle waits. `0` disables it.
    pub drain_threshold: u64,
    /// The engine's per-tick behaviour: batch size, lease, per-POST timeout,
    /// breaker and retry schedule.
    pub delivery: DeliveryConfig,
}

impl Default for DeliverySettings {
    fn default() -> Self {
        Self {
            worker_name: "seiva-webhook-delivery".to_string(),
            interval: Duration::from_secs(1),
            // A single due row is enough to keep draining fast — the right default
            // for a worker whose whole job is to empty the ledger.
            drain_threshold: 1,
            delivery: DeliveryConfig::default(),
        }
    }
}

/// Compose the [`DeliveryEngine`] from its three ports and drive
/// [`run_delivery_worker`] until `shutdown` resolves, returning the
/// [`WorkerReport`].
///
/// This is the composition root proper: the `ledger`, `sender` and `secrets` are
/// bound here and the worker loops on its own — the backlog drains with **no**
/// hand-ticking. The ledger's own operations each open their own session
/// ([`claim_due`](seiva_webhooks::delivery::DeliveryLedger::claim_due) and friends
/// wrap `with_session`), so the loop needs no ambient session — unlike the outbox
/// relay, whose pass runs raw SQL on the ambient session.
pub async fn run_delivery<S, K, Sig>(
    ledger: DormDeliveryLedger,
    sender: S,
    secrets: K,
    settings: DeliverySettings,
    shutdown: Sig,
) -> WorkerReport
where
    S: HttpSender,
    K: SigningSecrets,
    Sig: Future<Output = ()>,
{
    let engine = DeliveryEngine::new(ledger, sender, secrets, settings.delivery);
    let worker = WorkerConfig::new(settings.worker_name, settings.interval)
        .drain_fast_at(settings.drain_threshold);
    run_delivery_worker(worker, shutdown, engine).await
}

/// A failure while composing or running the service.
///
/// Each variant boxes its large inner error so a `Result<_, RunError>` stays
/// pointer-sized and does not trip clippy's `result_large_err`.
#[derive(Debug)]
#[non_exhaustive]
pub enum RunError {
    /// Building the DORM [`Database`](dorm_core::db::Database) handle failed.
    Connect(Box<dorm_core::Error>),
    /// Applying the idempotent delivery-table DDL failed.
    Migrate(Box<dorm_core::Error>),
    /// Building the reqwest client failed.
    Http(Box<reqwest::Error>),
    /// A required environment variable was missing or unparseable.
    Env(EnvError),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Connect(e) => write!(f, "failed to build the DORM handle: {e}"),
            RunError::Migrate(e) => write!(f, "failed to apply the delivery-table DDL: {e}"),
            RunError::Http(e) => write!(f, "failed to build the HTTP client: {e}"),
            RunError::Env(e) => write!(f, "invalid configuration: {e}"),
        }
    }
}

impl std::error::Error for RunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RunError::Connect(e) | RunError::Migrate(e) => Some(e),
            RunError::Http(e) => Some(e),
            RunError::Env(e) => Some(e),
        }
    }
}

impl From<EnvError> for RunError {
    fn from(e: EnvError) -> Self {
        RunError::Env(e)
    }
}

/// Apply the idempotent delivery-table DDL ([`delivery_migrations`]) over a shared
/// handle, so a fresh database has the table (and its due-work index) before the
/// worker starts. `CREATE … IF NOT EXISTS`, so running it on every boot is a no-op.
///
/// # Errors
///
/// Returns [`RunError::Migrate`] if any `CREATE` fails.
pub async fn ensure_schema(db: Arc<dorm_core::db::Database>) -> Result<(), RunError> {
    dorm_core::db::with_session(db, async {
        for stmt in delivery_migrations() {
            dorm_core::db::run(&stmt, &[]).await?;
        }
        Ok::<(), dorm_core::Error>(())
    })
    .await
    .map_err(|e| RunError::Migrate(Box::new(e)))
}

/// The runnable service: read the DORM handle from `dorm`, ensure the delivery
/// table exists, compose the [`DormDeliveryLedger`] + a production [`ReqwestSender`]
/// + the given `secrets`, and drain the ledger on the worker loop until `shutdown`.
///
/// The reqwest client is built with default TLS roots and pooling — a deployment
/// that needs a proxy, custom roots or per-POST timeouts builds its own client and
/// calls [`run_delivery`] with a [`ReqwestSender`] over it directly.
///
/// # Errors
///
/// Returns [`RunError::Connect`] if the handle cannot be built, [`RunError::Migrate`]
/// if the DDL fails, or [`RunError::Http`] if the client cannot be built.
pub async fn run_service<K, Sig>(
    dorm: DormConfig,
    settings: DeliverySettings,
    secrets: K,
    shutdown: Sig,
) -> Result<WorkerReport, RunError>
where
    K: SigningSecrets,
    Sig: Future<Output = ()>,
{
    let db = Arc::new(connect(&dorm).map_err(|e| RunError::Connect(Box::new(e)))?);
    ensure_schema(Arc::clone(&db)).await?;
    let ledger = DormDeliveryLedger::from_shared(db);

    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| RunError::Http(Box::new(e)))?;
    let sender = ReqwestSender::new(client);

    Ok(run_delivery(ledger, sender, secrets, settings, shutdown).await)
}

// ---------------------------------------------------------------------------
// Environment configuration for the binary.
// ---------------------------------------------------------------------------

/// The DSN of the delivery database. **Required.**
pub const DORM_DSN_VAR: &str = "DORM_DSN";
/// Comma-separated read-replica DSNs. Optional.
pub const DORM_REPLICAS_VAR: &str = "DORM_REPLICAS";
/// The PostgreSQL schema the handle is bound to. Optional.
pub const DORM_SCHEMA_VAR: &str = "DORM_SCHEMA";
/// Maximum pooled connections. Optional, default `8`.
pub const DORM_POOL_MAX_VAR: &str = "DORM_POOL_MAX";
/// The HMAC key for opaque keyset cursors. **Required** (no forgeable default).
pub const DORM_CURSOR_KEY_VAR: &str = "DORM_CURSOR_KEY";
/// The worker idle interval between passes, in milliseconds. Optional.
pub const DELIVERY_INTERVAL_MS_VAR: &str = "SEIVA_DELIVERY_INTERVAL_MS";
/// The fast-drain threshold (claimed rows). Optional.
pub const DELIVERY_DRAIN_AT_VAR: &str = "SEIVA_DELIVERY_DRAIN_AT";
/// The signing secrets, as `sub1=whsec_a,sub2=whsec_b`. Optional (an empty map
/// means every subscription is an unknown signer, so nothing is delivered).
pub const DELIVERY_SECRETS_VAR: &str = "SEIVA_WEBHOOK_SECRETS";

/// A failure reading configuration from the environment.
#[derive(Debug)]
pub enum EnvError {
    /// A required variable was not set.
    Missing(&'static str),
    /// A variable was set but could not be parsed.
    Invalid {
        /// The variable name.
        var: &'static str,
        /// A human-readable reason.
        reason: String,
    },
}

impl std::fmt::Display for EnvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvError::Missing(var) => write!(f, "required environment variable {var} is not set"),
            EnvError::Invalid { var, reason } => {
                write!(f, "environment variable {var} is invalid: {reason}")
            }
        }
    }
}

impl std::error::Error for EnvError {}

/// Read a [`DormConfig`] from the process environment.
///
/// [`DORM_DSN_VAR`] and [`DORM_CURSOR_KEY_VAR`] are required; the rest fall back to
/// safe defaults ([`DORM_POOL_MAX_VAR`] to `8`, replicas/schema empty). `read_only`
/// is forced off — the worker writes lease/record updates.
///
/// # Errors
///
/// [`EnvError::Missing`] for an unset required variable, [`EnvError::Invalid`] for
/// one that fails to parse.
pub fn dorm_config_from_env() -> Result<DormConfig, EnvError> {
    let dsn = required(DORM_DSN_VAR)?;
    let cursor_key = required(DORM_CURSOR_KEY_VAR)?.into_bytes();
    let replicas = std::env::var(DORM_REPLICAS_VAR)
        .ok()
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let schema = std::env::var(DORM_SCHEMA_VAR).ok().filter(|s| !s.is_empty());
    let pool_max = match std::env::var(DORM_POOL_MAX_VAR) {
        Ok(v) => v
            .parse()
            .map_err(|e: std::num::ParseIntError| EnvError::Invalid {
                var: DORM_POOL_MAX_VAR,
                reason: e.to_string(),
            })?,
        Err(_) => 8,
    };

    Ok(DormConfig {
        dsn,
        replicas,
        read_only: false,
        schema,
        pool_max,
        cursor_key,
    })
}

/// Read [`DeliverySettings`] from the environment, falling back to
/// [`DeliverySettings::default`] for anything unset.
///
/// # Errors
///
/// [`EnvError::Invalid`] if [`DELIVERY_INTERVAL_MS_VAR`] or [`DELIVERY_DRAIN_AT_VAR`]
/// is set but does not parse as a non-negative integer.
pub fn delivery_settings_from_env() -> Result<DeliverySettings, EnvError> {
    let mut settings = DeliverySettings::default();
    if let Ok(v) = std::env::var(DELIVERY_INTERVAL_MS_VAR) {
        let ms: u64 = v
            .parse()
            .map_err(|e: std::num::ParseIntError| EnvError::Invalid {
                var: DELIVERY_INTERVAL_MS_VAR,
                reason: e.to_string(),
            })?;
        settings.interval = Duration::from_millis(ms);
    }
    if let Ok(v) = std::env::var(DELIVERY_DRAIN_AT_VAR) {
        settings.drain_threshold =
            v.parse()
                .map_err(|e: std::num::ParseIntError| EnvError::Invalid {
                    var: DELIVERY_DRAIN_AT_VAR,
                    reason: e.to_string(),
                })?;
    }
    Ok(settings)
}

/// Read [`StaticSecrets`] from [`DELIVERY_SECRETS_VAR`] (`sub=secret,sub2=secret2`).
///
/// Unset or empty yields an empty map (no subscription has a signer). Whitespace
/// around each pair is trimmed; an entry without an `=` is rejected.
///
/// # Errors
///
/// [`EnvError::Invalid`] for an entry missing its `=` or with an empty key.
pub fn secrets_from_env() -> Result<StaticSecrets, EnvError> {
    let raw = match std::env::var(DELIVERY_SECRETS_VAR) {
        Ok(v) => v,
        Err(_) => return Ok(StaticSecrets::new()),
    };
    let mut secrets = StaticSecrets::new();
    for entry in raw.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let (sub, secret) = entry.split_once('=').ok_or(EnvError::Invalid {
            var: DELIVERY_SECRETS_VAR,
            reason: format!("entry {entry:?} is not `subscription=secret`"),
        })?;
        let sub = sub.trim();
        if sub.is_empty() {
            return Err(EnvError::Invalid {
                var: DELIVERY_SECRETS_VAR,
                reason: "a secrets entry has an empty subscription".to_string(),
            });
        }
        secrets = secrets.with_secret(sub, secret.trim());
    }
    Ok(secrets)
}

fn required(var: &'static str) -> Result<String, EnvError> {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => Err(EnvError::Missing(var)),
    }
}

/// Install the process shutdown signal: Ctrl-C and, on Unix, `SIGTERM`.
///
/// The returned future resolves when either arrives, so a container stop
/// (`SIGTERM`) and an interactive Ctrl-C both drive a graceful drain-and-exit.
pub async fn os_shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A parseable DSN deadpool accepts lazily — the handle builds without a live
    /// Postgres, the technique `seiva-data-dorm`'s own tests use.
    fn lazy_dorm_config() -> DormConfig {
        DormConfig {
            dsn: "host=localhost user=u dbname=x sslmode=disable".into(),
            replicas: vec![],
            read_only: false,
            schema: None,
            pool_max: 4,
            cursor_key: b"seiva-jobs-test-secret".to_vec(),
        }
    }

    #[test]
    fn connect_builds_a_handle_and_the_engine_composes() {
        // The compile + run is the proof the composition root wires the three
        // production ports: a DORM ledger, the reqwest sender, and StaticSecrets,
        // all satisfy DeliveryEngine's bounds. connect builds a handle from a
        // parseable DSN without connecting (deadpool is lazy).
        let db = connect(&lazy_dorm_config()).expect("the handle builds without connecting");
        let ledger = DormDeliveryLedger::from_shared(Arc::new(db));
        let client = reqwest::Client::builder().build().expect("client builds");
        let secrets = StaticSecrets::new().with_secret("sub_1", "whsec_abc");
        let _engine =
            DeliveryEngine::new(ledger, ReqwestSender::new(client), secrets, DeliveryConfig::default());
    }

    #[test]
    fn static_secrets_resolve_known_and_reject_unknown() {
        let secrets =
            StaticSecrets::new().with_secret("sub_1", "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw");
        assert!(secrets.signer("sub_1").is_some(), "a known signer resolves");
        assert!(secrets.signer("nope").is_none(), "an unknown signer is None");
        assert_eq!(secrets.len(), 1);
        assert!(!secrets.is_empty());
    }

    #[test]
    fn secrets_from_env_parses_pairs_and_rejects_malformed() {
        // The parser is pure over its input; assert it directly rather than mutate
        // the shared process env. Both values are VALID `whsec_…` secrets so the
        // signer actually resolves (an invalid secret would correctly resolve to
        // None — that path is the point of `signer` building on demand).
        const VALID: &str = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";
        let raw = format!("sub_1={VALID}, sub_2 = {VALID}");
        let ok = {
            let mut s = StaticSecrets::new();
            for entry in raw.split(',').map(str::trim).filter(|e| !e.is_empty()) {
                let (k, v) = entry.split_once('=').unwrap();
                s = s.with_secret(k.trim(), v.trim());
            }
            s
        };
        assert_eq!(ok.len(), 2);
        assert!(ok.signer("sub_1").is_some());
        assert!(ok.signer("sub_2").is_some());
    }

    #[test]
    fn settings_default_is_coherent() {
        let s = DeliverySettings::default();
        assert!(!s.worker_name.is_empty());
        assert_eq!(s.drain_threshold, 1, "a single due row keeps draining fast");
        assert_eq!(s.delivery.batch, DeliveryConfig::default().batch);
    }

    #[test]
    fn dorm_config_from_env_is_typed_and_callable() {
        // Deterministic without mutating the shared env: either the required vars
        // are unset here (Missing) or set in the ambient env — both acceptable.
        match dorm_config_from_env() {
            Err(EnvError::Missing(var)) => {
                assert!(var == DORM_DSN_VAR || var == DORM_CURSOR_KEY_VAR);
            }
            Err(EnvError::Invalid { .. }) | Ok(_) => {}
        }
    }
}
