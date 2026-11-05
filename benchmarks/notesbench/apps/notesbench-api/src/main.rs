//! Composition root for `notesbench`: the service's `main`. `seiva module new`
//! wires each module's router in at the managed anchor below; keep business
//! logic out.

use axum::Router;
use seiva_http::{router_with_modules, Health, HttpSecurity};
use seiva_server::{observability, serve_router, shutdown_signal, ServerConfig};
use std::process::ExitCode;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    // `seiva openapi` / `seiva proto` run this binary with SEIVA_EMIT set to
    // print THIS project's generated contract — from the facades its modules
    // register at link time — and exit, instead of serving. Done before
    // logging so stdout carries only the document.
    if let Ok(kind) = std::env::var("SEIVA_EMIT") {
        return emit_contract(&kind);
    }

    // Structured JSON logs to stdout before anything else.
    observability::init();

    let config = match ServerConfig::from_process_env() {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(%error, "invalid configuration");
            return ExitCode::FAILURE;
        }
    };

    let health = Health::starting();
    // Transport security posture: secure response headers on, same-origin
    // only (no CORS) until you add allowed origins.
    //
    // BENCHMARK NOTE: the scaffold default rate-limits every client IP to
    // 300 requests / 60s. A load generator hammering from one IP would see
    // almost all requests answered 429 after the first 300, which measures
    // the rate limiter, not request throughput. The FastAPI/Django apps in
    // this harness have no rate limiter, so we switch Seiva's off here to
    // keep the comparison fair. This is a security control, not a speed
    // feature; it does not reflect how fast the framework serves a request.
    let security = HttpSecurity {
        rate_limit: None,
        ..HttpSecurity::default()
    };
    // No authenticator wired yet, so guarded routes answer 403; supply an
    // `Arc<dyn seiva_http::Authenticator>` (see `seiva-identity`) to admit
    // callers. `None` leaves public routes serving.
    let auth = None;
    // Module routers are merged here, BEFORE the framework layers wrap them —
    // `router_with_modules` merges then layers, so authn, security headers
    // and the rate limit apply to module routes too. `seiva module new`
    // inserts each module's merge at the anchor below.
    let modules = Router::new();
    let modules = modules.merge(notes_interface::notes_router(
        notes_interface::NotesController::new(
            notes_infrastructure::InMemoryRecordRepository::new(),
            notes_infrastructure::CounterIdGenerator::default(),
        ),
    ));
    // seiva:module-merges (managed by the seiva CLI)
    let app = router_with_modules(health.clone(), &security, auth, modules);

    tracing::info!(env = %config.env, http = %config.http_addr, "notesbench-api starting");
    match serve_router(config.http_addr, app, health, shutdown_signal()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "serve failed");
            ExitCode::FAILURE
        }
    }
}

/// Emits this project's generated contract from the facades linked into this
/// binary (the modules merged above), then exits. Driven by `seiva openapi`.
fn emit_contract(kind: &str) -> ExitCode {
    let facades = delonix_seiva::contract::registered_facades();
    match kind {
        "openapi" => {
            let doc = seiva_openapi::openapi_document(
                seiva_openapi::ApiInfo {
                    title: "notesbench API".to_string(),
                    version: "0.1.0".to_string(),
                },
                &facades,
            );
            match serde_json::to_string_pretty(&doc) {
                Ok(json) => {
                    println!("{json}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("seiva: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        "proto" => {
            for facade in &facades {
                println!("{}", seiva_grpc::proto_document(facade));
            }
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("seiva: unknown SEIVA_EMIT value `{other}` (expected `openapi` or `proto`)");
            ExitCode::FAILURE
        }
    }
}
