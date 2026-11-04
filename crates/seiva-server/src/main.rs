//! The `seiva-server` binary: a thin `main` over the [`seiva_server`] library.
//!
//! It reads [`ServerConfig`] from the environment, installs a shutdown signal
//! (Ctrl-C, and `SIGTERM` on unix), and runs the HTTP server ([`serve`]) and the
//! background worker host ([`run_workers`]) concurrently, sharing the shutdown.
//! On a configuration error it prints every problem and exits non-zero; on a
//! serve error it reports it and exits non-zero.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use seiva_server::{observability, run_workers, serve, ServerConfig};

/// Resolves once the process is asked to shut down: Ctrl-C (SIGINT) on every
/// platform, or `SIGTERM` on unix (the signal an orchestrator sends to stop a
/// container). Each caller builds its own future, so the server and the worker
/// host can each await shutdown independently without sharing a channel.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut term = signal(SignalKind::terminate()).expect("install a SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

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

    // Serve and the worker host run concurrently, each with its own shutdown
    // future over the same OS signals.
    let (served, reports) = tokio::join!(
        serve(config, shutdown_signal()),
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
