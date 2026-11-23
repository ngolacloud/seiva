//! The `seiva-jobs` binary: run the ADR 0034 webhook delivery job as a service.
//!
//! It reads a [`DormConfig`](seiva_data_dorm::DormConfig) and
//! [`DeliverySettings`](seiva_jobs::DeliverySettings) from the environment,
//! installs a Ctrl-C + `SIGTERM` shutdown, composes the DORM ledger + the reqwest
//! sender + the configured signing secrets, and drains the delivery ledger on the
//! shared worker loop until a signal arrives.
//!
//! # Environment
//!
//! DORM (from [`seiva_jobs::dorm_config_from_env`]): `DORM_DSN` (required),
//! `DORM_CURSOR_KEY` (required), `DORM_REPLICAS`, `DORM_SCHEMA`, `DORM_POOL_MAX`.
//!
//! Worker (from [`seiva_jobs::delivery_settings_from_env`]):
//! `SEIVA_DELIVERY_INTERVAL_MS`, `SEIVA_DELIVERY_DRAIN_AT`.
//!
//! Secrets (from [`seiva_jobs::secrets_from_env`]): `SEIVA_WEBHOOK_SECRETS` as
//! `sub1=whsec_a,sub2=whsec_b`.

#![forbid(unsafe_code)]

use seiva_jobs::{
    delivery_settings_from_env, dorm_config_from_env, os_shutdown, run_service, secrets_from_env,
};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!(target: "seiva_jobs", "starting the webhook delivery worker");

    let dorm = dorm_config_from_env()?;
    let settings = delivery_settings_from_env()?;
    let secrets = secrets_from_env()?;
    if secrets.is_empty() {
        tracing::warn!(
            target: "seiva_jobs",
            "no signing secrets configured ({}); every delivery will fail as an unknown signer",
            seiva_jobs::DELIVERY_SECRETS_VAR,
        );
    }

    let report = run_service(dorm, settings, secrets, os_shutdown()).await?;

    tracing::info!(
        target: "seiva_jobs",
        passes = report.passes,
        processed = report.processed,
        errors = report.errors,
        "the delivery worker stopped; service shut down cleanly",
    );
    Ok(())
}
