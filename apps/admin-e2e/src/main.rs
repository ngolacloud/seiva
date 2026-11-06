//! Composition root for the admin browser E2E (§10). Serves the `accounts`
//! example module — which registers an admin resource (ADR 0009) — with the
//! console mounted into the pre-layer `modules` router (ADR 0008/0010) behind a
//! token authenticator, so a real browser can authenticate, list the authorized
//! resource and drive the create form.
//!
//! A valid bearer is `clerk-token` → a principal holding `accounts.read`
//! (so the resource is visible) and `accounts.create` (so the create form runs).
//! Bind address from `ADMIN_E2E_ADDR` (default `127.0.0.1:8099`). Runs until
//! killed; the E2E runner starts it, drives Chrome, then stops it.

use std::net::SocketAddr;
use std::sync::Arc;

use seiva_http::{
    router_with_modules, Authenticator, Health, HttpSecurity, Principal, TokenAuthenticator,
};
use seiva_server::serve_router;

#[tokio::main]
async fn main() {
    // The accounts module router (its facade registers the "accounts" admin
    // resource at link time), then the console mounted INTO modules so it is
    // wrapped by the framework layers (authn populates the Principal).
    let repo = accounts_infrastructure::InMemoryCredentialRepository::new();
    let modules =
        accounts_interface::accounts_router(accounts_interface::AccountsController::new(repo));
    let modules = seiva_console_admin::mount_dx(modules);

    let health = Health::starting();
    health.set_ready();

    let auth: Arc<dyn Authenticator> = Arc::new(TokenAuthenticator::new().with_token(
        "clerk-token",
        Principal::new("clerk", ["accounts.read", "accounts.create"]),
    ));

    let app = router_with_modules(
        health.clone(),
        &HttpSecurity::default(),
        Some(auth),
        modules,
    );

    let addr: SocketAddr = std::env::var("ADMIN_E2E_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8099".to_owned())
        .parse()
        .expect("ADMIN_E2E_ADDR must be a host:port");

    eprintln!("admin-e2e: serving http://{addr}/admin (token: clerk-token)");
    if let Err(error) = serve_router(addr, app, health, std::future::pending::<()>()).await {
        eprintln!("admin-e2e: serve failed: {error}");
        std::process::exit(1);
    }
}
