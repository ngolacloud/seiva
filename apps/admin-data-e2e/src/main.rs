//! Runnable composition root for the data-layer admin proof: the `Catalog` CRUD
//! module (over real PostgreSQL via DORM) + the console, behind a token
//! authenticator. A browser (or `curl`) drives row CRUD through the console at
//! `/admin`. DSN from `ADMIN_DATA_E2E_DSN` (or `SEIVA_DORM_TEST_DSN`); bind from
//! `ADMIN_DATA_E2E_ADDR` (default `127.0.0.1:8098`). `clerk-token` holds every
//! `catalog.*` permission.

use std::net::SocketAddr;
use std::sync::Arc;

use admin_data_e2e::{catalog_router, catalog_table_ddl, CatalogController};
use seiva_data_dorm::{
    connect, idempotency_migrations, DormConfig, DormIdempotencyStore, DormUnitOfWork,
};
use seiva_http::{
    router_with_modules, router_with_modules_distributed, Authenticator, DistributedRateLimit,
    Health, HttpSecurity, Principal, RateLimit, TokenAuthenticator,
};
use seiva_ports::{BoxError, Scope, UnitOfWork};
use seiva_ratelimit_redis::RedisRateLimitStore;
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

    // Create the demo table + the idempotency table (both idempotent DDL) before
    // serving. In the containerised multi-replica deployment these are pre-created
    // by the PostgreSQL init script, so this is a no-op there.
    let mut ddl = vec![catalog_table_ddl().to_owned()];
    ddl.extend(idempotency_migrations());
    uow.transaction(Scope::Global, move || async move {
        for stmt in &ddl {
            dorm_core::db::execute(stmt, &[])
                .await
                .map_err(|e| Box::new(e) as BoxError)?;
        }
        Ok(())
    })
    .await?;

    let modules = catalog_router(CatalogController::new(uow, Arc::new(DormIdempotencyStore)));
    let modules = seiva_console_admin::mount(modules);

    let health = Health::starting();
    health.set_ready();
    let auth: Arc<dyn Authenticator> = Arc::new(TokenAuthenticator::new().with_token(
        "clerk-token",
        Principal::in_tenant(
            "clerk",
            "acme",
            [
                "catalog.read",
                "catalog.create",
                "catalog.change",
                "catalog.delete",
                "catalog.cost.view",
            ],
        ),
    ));
    // Rate limiting stays ON at the secure default (300 req/min per IP). The
    // production-load benchmark raises it via `ADMIN_DATA_E2E_RATE_LIMIT` (requests
    // per minute) so it can measure the auth+DB work path instead of the limiter's
    // load-shedding; absent the env var the deployment keeps the protective default.
    let mut security = HttpSecurity::default();
    let rate = if let Ok(rpm) = std::env::var("ADMIN_DATA_E2E_RATE_LIMIT") {
        let max_requests: u32 = rpm.parse().map_err(|_| {
            "ADMIN_DATA_E2E_RATE_LIMIT must be a positive integer (requests/minute)"
        })?;
        RateLimit {
            max_requests,
            window: std::time::Duration::from_secs(60),
        }
    } else {
        RateLimit::default()
    };
    security.rate_limit = Some(rate);
    // Behind a load balancer the per-client budget must be ONE across replicas, so
    // key on the forwarded client IP and count in a shared store. When
    // `ADMIN_DATA_E2E_REDIS_URL` is set the replica uses the DISTRIBUTED limiter
    // (Redis, ADR 0015); without it, the per-process limiter above applies.
    let app = if let Ok(redis_url) = std::env::var("ADMIN_DATA_E2E_REDIS_URL") {
        security.trust_forwarded_for = true;
        let store = Arc::new(RedisRateLimitStore::connect(&redis_url).await?);
        let dist = DistributedRateLimit::new(store, rate).trusting_forwarded_for(true);
        eprintln!(
            "admin-data-e2e: distributed rate limit via Redis ({redis_url}), {} req/min",
            rate.max_requests
        );
        router_with_modules_distributed(health.clone(), &security, Some(auth), modules, dist)
    } else {
        router_with_modules(health.clone(), &security, Some(auth), modules)
    };

    let addr: SocketAddr = std::env::var("ADMIN_DATA_E2E_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8098".to_owned())
        .parse()
        .map_err(|e| Box::new(e) as BoxError)?;

    eprintln!("admin-data-e2e: serving http://{addr}/admin (token: clerk-token)");
    serve_router(addr, app, health, std::future::pending::<()>())
        .await
        .map_err(|e| Box::new(e) as BoxError)
}
