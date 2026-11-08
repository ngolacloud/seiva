//! Composition root for the `shop` tutorial — the binary shell.
//!
//! It is a thin wrapper over [`shop_api::build_app`], which composes the five
//! INDEPENDENT shop modules into one HTTP service and wires the live
//! `orders.placed → inventory.reserved` choreography (ADR 0021 Fase 3). All the
//! wiring lives in the library target so the e2e test drives the same composition;
//! here we only serve it.
//!
//! The service serves, with no per-developer wiring:
//!
//! - `/health/{live,ready}` — the framework health endpoints.
//! - `/openapi.json` — the OpenAPI 3.1 document, generated from every facade linked
//!   into this binary (so every shop route appears automatically).
//! - `/docs` — Swagger UI over that document (FastAPI `/docs` parity), plus
//!   `/redoc` and `/scalar`.
//! - `/admin` — the optional console (ADR 0008/0009).
//!
//! A valid demo bearer is `shop-token` → a principal holding every shop permission.
//! Bind address from `SHOP_API_ADDR` (default `127.0.0.1:8090`). Runs until killed.
//!
//! The in-process outbox drain (the choreography's driver) is exercised by
//! `tests/choreography_e2e.rs`; this binary serves the HTTP surface.

use std::net::SocketAddr;

use seiva_server::serve_router;
use shop_api::build_app;

#[tokio::main]
async fn main() {
    let app = build_app();

    let addr: SocketAddr = std::env::var("SHOP_API_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8090".to_owned())
        .parse()
        .expect("SHOP_API_ADDR must be a host:port");

    eprintln!("shop-api: serving http://{addr} (docs: /docs, admin: /admin, token: shop-token)");
    if let Err(error) =
        serve_router(addr, app.router, app.health, std::future::pending::<()>()).await
    {
        eprintln!("shop-api: serve failed: {error}");
        std::process::exit(1);
    }
}
