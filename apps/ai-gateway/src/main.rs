//! `seiva-ai-gateway` — serve the AI gateway from one TOML file.
//!
//! ```text
//! seiva-ai-gateway serve <config.toml>   serve until Ctrl-C (or SIGTERM)
//! seiva-ai-gateway check <config.toml>   validate the file and exit
//! seiva-ai-gateway hash-key              read a key on stdin, print its SHA-256
//! ```
//!
//! Provider keys come from the environment (the `credential` of each provider,
//! `ANTHROPIC_API_KEY` by default); the file only holds the SHA-256 of each
//! gateway key.

#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;

use ai_gateway::{build, Config, JsonLinesAudit, SystemClock};
use seiva_ai_gateway::server::hash_key;
use seiva_llm::EnvCredentials;
use seiva_llm_reqwest::ReqwestTransport;
use tokio::io::AsyncReadExt;

const USAGE: &str = "usage: seiva-ai-gateway serve <config.toml> | check <config.toml> | hash-key";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["serve", path] => serve(path, false).await,
        ["check", path] => serve(path, true).await,
        ["hash-key"] => hash_stdin().await,
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("seiva-ai-gateway: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn hash_stdin() -> Result<(), String> {
    let mut key = String::new();
    tokio::io::stdin()
        .read_to_string(&mut key)
        .await
        .map_err(|e| e.to_string())?;
    let key = key.trim();
    if key.is_empty() {
        return Err("no key on stdin".into());
    }
    println!("{}", hash_key(key));
    Ok(())
}

async fn serve(path: &str, check_only: bool) -> Result<(), String> {
    let config = Config::load(std::path::Path::new(path))?;
    let transport = Arc::new(ReqwestTransport::with_defaults().map_err(|e| e.to_string())?);
    let audit = Arc::new(JsonLinesAudit::open(&config.audit)?);
    let health = seiva_http::Health::starting();
    let app = build(
        &config,
        transport,
        Arc::new(EnvCredentials),
        audit,
        Arc::new(SystemClock),
        health.clone(),
    )?;
    if check_only {
        println!(
            "ok: {} provider(s), {} route(s), {} key(s)",
            config.providers.len(),
            config.routes.len(),
            config.keys.len()
        );
        return Ok(());
    }
    eprintln!("seiva-ai-gateway: listening on {}", config.listen);
    seiva_server::serve_router(config.listen, app, health, shutdown())
        .await
        .map_err(|e| e.to_string())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut term =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => s,
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                    return;
                }
            };
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
