//! Runnable composition root for the data-layer admin proof: the `Catalog` CRUD
//! module (over real PostgreSQL via DORM) + the console, behind a token
//! authenticator. A browser (or `curl`) drives row CRUD through the console at
//! `/admin`. DSN from `ADMIN_DATA_E2E_DSN` (or `SEIVA_DORM_TEST_DSN`); bind from
//! `ADMIN_DATA_E2E_ADDR` (default `127.0.0.1:8098`). `clerk-token` holds every
//! `catalog.*` permission.

use std::net::SocketAddr;
use std::sync::Arc;

use admin_data_e2e::{catalog_router, catalog_table_ddl, CatalogController};
use seiva_data_dorm::{connect, DormConfig, DormUnitOfWork};
use seiva_http::{
    router_with_modules, Authenticator, Health, HttpSecurity, Principal, TokenAuthenticator,
};
use seiva_ports::{BoxError, UnitOfWork};
use seiva_server::serve_router;

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let dsn = std::env::var("ADMIN_DATA_E2E_DSN")
        .or_else(|_| std::env::var("SEIVA_DORM_TEST_DSN"))
        .map_err(|_| "set ADMIN_DATA_E2E_DSN (or SEIVA_DORM_TEST_DSN) to a Postgres DSN")?;

    let uow = Arc::new(DormUnitOfWork::new(connect(&DormConfig {
        dsn,
        replicas: vec![],
        read_only: false,
        schema: None,
        pool_max: 8,
        cursor_key: b"admin-data-e2e-secret".to_vec(),
    })?));

    // Create the demo table (idempotent) before serving.
    uow.transaction(|| async {
        dorm_core::db::execute(catalog_table_ddl(), &[])
            .await
            .map(|_| ())
            .map_err(|e| Box::new(e) as BoxError)
    })
    .await?;

    let modules = catalog_router(CatalogController::new(uow));
    let modules = seiva_console_admin::mount(modules);

    let health = Health::starting();
    health.set_ready();
    let auth: Arc<dyn Authenticator> = Arc::new(TokenAuthenticator::new().with_token(
        "clerk-token",
        Principal::new(
            "clerk",
            [
                "catalog.read",
                "catalog.create",
                "catalog.change",
                "catalog.delete",
            ],
        ),
    ));
    let app = router_with_modules(
        health.clone(),
        &HttpSecurity::default(),
        Some(auth),
        modules,
    );

    let addr: SocketAddr = std::env::var("ADMIN_DATA_E2E_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8098".to_owned())
        .parse()
        .map_err(|e| Box::new(e) as BoxError)?;

    eprintln!("admin-data-e2e: serving http://{addr}/admin (token: clerk-token)");
    serve_router(addr, app, health, std::future::pending::<()>())
        .await
        .map_err(|e| Box::new(e) as BoxError)
}
