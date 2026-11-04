//! The `dorm-e2e` binary: the runnable end-to-end proof of Seiva's `dorm` path.
//!
//! It reads a [`ServerConfig`](seiva_server::ServerConfig) and a
//! [`DormConfig`](seiva_data_dorm::DormConfig) from the environment, installs a
//! Ctrl-C + `SIGTERM` shutdown, and runs the HTTP server and the transactional-
//! outbox → broker relay worker concurrently under that one shutdown.
//!
//! # Environment
//!
//! HTTP (from [`seiva_server::ServerConfig::from_process_env`]):
//! `SEIVA_HTTP_ADDR` or `PORT`, `APP_ENV`, `SEIVA_EXPOSE_ERROR_DETAILS`.
//!
//! DORM (from [`dorm_e2e::dorm_config_from_env`]):
//! `DORM_DSN` (required), `DORM_CURSOR_KEY` (required), `DORM_REPLICAS`,
//! `DORM_READ_ONLY`, `DORM_SCHEMA`, `DORM_POOL_MAX`.
//!
//! Relay (from [`dorm_e2e::relay_settings_from_env`]):
//! `DORM_RELAY_CONSUMER`, `DORM_RELAY_TOPIC`, `DORM_RELAY_INTERVAL_MS`.
//!
//! The broker is stood in for by [`dorm_e2e::LoggingPublisher`], so the binary runs
//! end to end without a live broker; swap it for a `seiva-messaging` backend
//! (NATS/AMQP/Kafka) to forward for real.

#![forbid(unsafe_code)]

use dorm_e2e::{dorm_config_from_env, os_shutdown, relay_settings_from_env, run, LoggingPublisher};
use seiva_server::ServerConfig;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!(target: "dorm_e2e", "starting the DORM end-to-end service");

    let server = ServerConfig::from_process_env()?;
    let dorm = dorm_config_from_env()?;
    let relay = relay_settings_from_env()?;
    let publisher = LoggingPublisher::new();

    let report = run(server, dorm, relay, publisher, os_shutdown()).await?;

    tracing::info!(
        target: "dorm_e2e",
        passes = report.passes,
        processed = report.processed,
        errors = report.errors,
        "the relay worker stopped; service shut down cleanly",
    );
    Ok(())
}
