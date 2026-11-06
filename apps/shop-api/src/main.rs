//! Composition root for the `shop` tutorial.
//!
//! It composes the five INDEPENDENT shop modules — catalog, inventory, orders,
//! payments, notifications — into one running HTTP service. Each module
//! contributes its generated controller router (keyed off its `#[facade]`
//! descriptor) and an in-memory repository; the modules share no crate, so the
//! monolith→microservices story is told by composition, not coupling.
//!
//! The service serves, with no per-developer wiring:
//!
//! - `/health/{live,ready}` — the framework health endpoints.
//! - `/openapi.json` — the OpenAPI 3.1 document, generated from every facade
//!   linked into this binary (so every shop route appears automatically).
//! - `/docs` — Swagger UI over that document (FastAPI `/docs` parity), plus
//!   `/redoc` and `/scalar`.
//! - `/admin` — the optional console (ADR 0008), mounted INTO the module router
//!   so the framework layers wrap it; it lists each module's registered admin
//!   resource (ADR 0009).
//!
//! A valid demo bearer is `shop-token` → a principal holding every shop
//! permission, so the console shows all resources and the create forms run.
//! Bind address from `SHOP_API_ADDR` (default `127.0.0.1:8090`). Runs until
//! killed.

use std::net::SocketAddr;
use std::sync::Arc;

use seiva_http::{
    router_with_modules, Authenticator, Health, HttpSecurity, Principal, TokenAuthenticator,
};
use seiva_server::serve_router;

use shop_catalog_interface::{catalog_router, CatalogController};
use shop_inventory_interface::{inventory_router, InventoryController};
use shop_notifications_interface::{notifications_router, NotificationsController};
use shop_orders_interface::{orders_router, OrdersController};
use shop_payments_interface::{payments_router, PaymentsController};

#[tokio::main]
async fn main() {
    // Each module's controller over its own in-memory repository. The generated
    // routers merge into one `modules` router; each facade registers itself in
    // the link-time inventory, so `/openapi.json` discovers all five.
    let modules = catalog_router(CatalogController::new(
        shop_catalog_infrastructure::InMemoryProductRepository::new(),
    ))
    .merge(inventory_router(InventoryController::new(
        shop_inventory_infrastructure::InMemoryStockRepository::new(),
    )))
    .merge(orders_router(OrdersController::new(
        shop_orders_infrastructure::InMemoryOrderRepository::new(),
    )))
    .merge(payments_router(PaymentsController::new(
        shop_payments_infrastructure::InMemoryPaymentRepository::new(),
    )))
    .merge(notifications_router(NotificationsController::new(
        shop_notifications_infrastructure::InMemoryNotificationRepository::new(),
    )));

    // Mount the console INTO modules so it is wrapped by the framework layers
    // (authn populates the Principal the console's exposure checks read).
    let modules = seiva_console_admin::mount(modules);

    let health = Health::starting();
    health.set_ready();

    // One demo token holding every shop permission. Permissions are matched
    // verbatim by the guard (no wildcards), so each `<module>.<action>` the
    // facades require is listed explicitly.
    let auth: Arc<dyn Authenticator> = Arc::new(TokenAuthenticator::new().with_token(
        "shop-token",
        Principal::new(
            "shopkeeper",
            [
                "catalog.read",
                "catalog.manage",
                "inventory.read",
                "inventory.manage",
                "orders.read",
                "orders.manage",
                "payments.read",
                "payments.charge",
                "notifications.read",
                "notifications.send",
            ],
        ),
    ));

    // CORS for a separately-deployed frontend (Part 11): list the storefront's
    // origin(s) in SHOP_API_CORS_ORIGINS (comma-separated). Empty = same-origin
    // only (the secure default); non-empty mounts the CORS layer for exactly
    // those origins.
    let security = match std::env::var("SHOP_API_CORS_ORIGINS") {
        Ok(origins) if !origins.trim().is_empty() => {
            HttpSecurity::with_cors_origins(origins.split(',').map(|o| o.trim().to_owned()))
        }
        _ => HttpSecurity::default(),
    };

    let app = router_with_modules(health.clone(), &security, Some(auth), modules);

    let addr: SocketAddr = std::env::var("SHOP_API_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8090".to_owned())
        .parse()
        .expect("SHOP_API_ADDR must be a host:port");

    eprintln!("shop-api: serving http://{addr} (docs: /docs, admin: /admin, token: shop-token)");
    if let Err(error) = serve_router(addr, app, health, std::future::pending::<()>()).await {
        eprintln!("shop-api: serve failed: {error}");
        std::process::exit(1);
    }
}
