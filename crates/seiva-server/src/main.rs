//! The `seiva-server` binary: a thin `main` over the [`seiva_server`] library.
//!
//! It reads [`ServerConfig`] from the environment, installs a shutdown signal
//! (Ctrl-C, and `SIGTERM` on unix), and runs the HTTP server ([`serve`]) and the
//! background worker host ([`run_workers`]) concurrently, sharing the shutdown.
//! On a configuration error it prints every problem and exits non-zero; on a
//! serve error it reports it and exits non-zero.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use seiva_server::{observability, run_workers, shutdown_signal, Server, ServerConfig};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    // Structured JSON logs to stdout before anything else, so even a config error
    // is one parseable line for the platform to collect.
    observability::init();

    let config = match ServerConfig::from_process_env() {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(%error, "invalid configuration");
            return ExitCode::FAILURE;
        }
    };

    tracing::info!(env = %config.env, http = %config.http_addr, "starting");

    #[cfg_attr(not(feature = "metrics"), allow(unused_mut))]
    let mut server = Server::new(config);

    // Opt in to the Prometheus `/metrics` exporter when `SEIVA_METRICS_ADDR` is set
    // (ADR 0027), on its own bind. Only compiled with the `metrics` feature; a bad
    // address fails the boot rather than serving without the exporter.
    #[cfg(feature = "metrics")]
    {
        match std::env::var(seiva_server::METRICS_ADDR_VAR) {
            Ok(raw) if !raw.trim().is_empty() => match raw.trim().parse() {
                Ok(addr) => server = server.with_metrics(addr),
                Err(error) => {
                    tracing::error!(%error, var = seiva_server::METRICS_ADDR_VAR, "invalid metrics address");
                    return ExitCode::FAILURE;
                }
            },
            _ => {}
        }
    }

    // Serve and the worker host run concurrently, each with its own shutdown
    // future over the same OS signals.
    let (served, reports) = tokio::join!(
        server.serve(shutdown_signal()),
        run_workers(shutdown_signal())
    );

    for report in &reports {
        tracing::info!(
            passes = report.passes,
            processed = report.processed,
            errors = report.errors,
            "worker stopped"
        );
    }

    match served {
        Ok(()) => {
            tracing::info!("stopped cleanly");
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!(%error, "serve failed");
            ExitCode::FAILURE
        }
    }
}
