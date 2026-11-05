//! Live proof that the `Catalog` module's five CRUD operations work end to end
//! over a real PostgreSQL through the DORM gateway, **tenant-scoped** and with a
//! **masked field** (ADR 0006/0009). Gated on `SEIVA_DORM_TEST_DSN`; skips when
//! unset. It drives the `CatalogController` operations directly (the same ones the
//! console calls over HTTP), passing a `&Ctx` built from a `Principal` — exactly
//! what the generated controller injects from the request.

use std::sync::Arc;

use admin_data_e2e::{
    catalog_table_ddl, Catalog, CatalogController, CreateItem, DeleteItem, UpdateItem,
};
use seiva_data_dorm::{
    connect, idempotency_migrations, DormConfig, DormIdempotencyStore, DormUnitOfWork,
};
use seiva_http::{Ctx, IdempotencyKey, Principal};
use seiva_ports::{BoxError, UnitOfWork};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_catalog_crud_tenant_scoped_and_masked() {
    let Ok(dsn) = std::env::var("SEIVA_DORM_TEST_DSN") else {
        eprintln!("skipping: set SEIVA_DORM_TEST_DSN to run the live catalog CRUD proof");
        return;
    };
    let uow = Arc::new(DormUnitOfWork::new(
        connect(&DormConfig {
            dsn,
            replicas: vec![],
            read_only: false,
            schema: None,
            pool_max: 4,
            cursor_key: b"admin-data-e2e-secret".to_vec(),
        })
        .expect("connect to the test database"),
    ));

    uow.transaction(|| async {
        dorm_core::db::execute("DROP TABLE IF EXISTS catalog_items", &[])
            .await
            .map_err(|e| Box::new(e) as BoxError)?;
        dorm_core::db::execute(catalog_table_ddl(), &[])
            .await
            .map(|_| ())
            .map_err(|e| Box::new(e) as BoxError)
    })
    .await
    .expect("table setup");

    let catalog = CatalogController::new(Arc::clone(&uow), Arc::new(DormIdempotencyStore));

    // Two tenants; acme's principal also holds the cost-view permission.
    let acme = Ctx::new(Principal::in_tenant(
        "clerk",
        "acme",
        [
            "catalog.read",
            "catalog.create",
            "catalog.change",
            "catalog.delete",
            "catalog.cost.view",
        ],
    ));
    let acme_no_cost = Ctx::new(Principal::in_tenant("clerk2", "acme", ["catalog.read"]));
    let globex = Ctx::new(Principal::in_tenant(
        "other",
        "globex",
        ["catalog.read", "catalog.create"],
    ));

    // CREATE in acme, with a cost.
    let created = catalog
        .create(
            &acme,
            CreateItem {
                name: "widget".into(),
                price: 100,
                cost: Some(60),
            },
        )
        .await
        .expect("create");
    let id = created.id;
    assert_eq!(
        created.cost,
        Some(60),
        "the creator (cost.view) sees the cost"
    );

    // MASKING: acme_no_cost reads the same row but the cost is masked away.
    let masked = catalog.get(&acme_no_cost, id).await.expect("get masked");
    assert_eq!(masked.name, "widget");
    assert_eq!(
        masked.cost, None,
        "a caller without cost.view must not receive the cost"
    );

    // TENANT ISOLATION: globex cannot see or touch acme's row.
    assert!(
        catalog.get(&globex, id).await.is_err(),
        "another tenant cannot read the row"
    );
    assert!(
        catalog.list(&globex).await.expect("globex list").is_empty(),
        "globex sees no acme rows"
    );
    // globex has its own, separate row.
    catalog
        .create(
            &globex,
            CreateItem {
                name: "gx".into(),
                price: 5,
                cost: None,
            },
        )
        .await
        .expect("globex create");
    assert_eq!(
        catalog.list(&acme).await.expect("acme list").len(),
        1,
        "acme still sees only its own"
    );
    assert_eq!(catalog.list(&globex).await.expect("globex list").len(), 1);

    // UPDATE + DELETE within acme.
    let updated = catalog
        .update(
            &acme,
            UpdateItem {
                id,
                name: "widget2".into(),
                price: 150,
                cost: Some(70),
            },
        )
        .await
        .expect("update");
    assert_eq!(updated.name, "widget2");
    assert_eq!(updated.cost, Some(70));
    // globex cannot delete acme's row (tenant scope): the delete affects nothing,
    // so acme's row survives.
    catalog
        .delete(&globex, DeleteItem { id })
        .await
        .expect("globex delete is a no-op row-wise");
    assert!(
        catalog.get(&acme, id).await.is_ok(),
        "cross-tenant delete must not remove the row"
    );
    // acme deletes its own row.
    catalog
        .delete(&acme, DeleteItem { id })
        .await
        .expect("delete");
    assert!(catalog.get(&acme, id).await.is_err(), "deleted");

    // Cleanup.
    uow.transaction(|| async {
        dorm_core::db::execute("DROP TABLE catalog_items", &[])
            .await
            .map(|_| ())
            .map_err(|e| Box::new(e) as BoxError)
    })
    .await
    .expect("cleanup");
}

/// Live proof that a retried `create` carrying the same `Idempotency-Key` inserts
/// exactly once (the retry replays the stored response), and that reusing the key
/// with different content is a `409` (ADR 0011). Drives the controller directly
/// with a `&Ctx` carrying the key, exactly what the generated handler injects from
/// the request header. Gated on `SEIVA_DORM_TEST_DSN`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_catalog_create_is_idempotent() {
    let Ok(dsn) = std::env::var("SEIVA_DORM_TEST_DSN") else {
        eprintln!("skipping: set SEIVA_DORM_TEST_DSN to run the idempotency proof");
        return;
    };
    let uow = Arc::new(DormUnitOfWork::new(
        connect(&DormConfig {
            dsn,
            replicas: vec![],
            read_only: false,
            schema: None,
            pool_max: 4,
            cursor_key: b"admin-data-e2e-secret".to_vec(),
        })
        .expect("connect to the test database"),
    ));

    // Fresh tables: the catalogue and the idempotency store.
    uow.transaction(|| async {
        dorm_core::db::execute("DROP TABLE IF EXISTS catalog_items", &[])
            .await
            .map_err(|e| Box::new(e) as BoxError)?;
        dorm_core::db::execute("DROP TABLE IF EXISTS dorm_idempotency", &[])
            .await
            .map_err(|e| Box::new(e) as BoxError)?;
        dorm_core::db::execute(catalog_table_ddl(), &[])
            .await
            .map_err(|e| Box::new(e) as BoxError)?;
        for stmt in idempotency_migrations() {
            dorm_core::db::execute(&stmt, &[])
                .await
                .map_err(|e| Box::new(e) as BoxError)?;
        }
        Ok(())
    })
    .await
    .expect("table setup");

    let catalog = CatalogController::new(Arc::clone(&uow), Arc::new(DormIdempotencyStore));
    let principal =
        Principal::in_tenant("clerk", "acme", ["catalog.create", "catalog.read", "catalog.cost.view"]);
    let key = IdempotencyKey::parse("order-42").expect("valid key");
    let ctx = Ctx::new(principal).with_request("POST", "/v1/catalogs", Some(key.clone()));

    let body = CreateItem {
        name: "widget".into(),
        price: 100,
        cost: Some(60),
    };

    // First call inserts.
    let first = catalog.create(&ctx, body.clone()).await.expect("first create");
    // Retry with the SAME key and SAME content replays the stored response.
    let replay = catalog
        .create(&ctx, body.clone())
        .await
        .expect("retry replays");
    assert_eq!(first, replay, "a retry returns the identical stored response");

    // Exactly one row exists for the tenant.
    let count = catalog.list(&ctx).await.expect("list").len();
    assert_eq!(count, 1, "the retried create inserted exactly once");

    // Reusing the key with DIFFERENT content is a 409 conflict (not a second row).
    let conflicting = CreateItem {
        name: "widget".into(),
        price: 999, // different content under the same key
        cost: Some(60),
    };
    let err = catalog
        .create(&ctx, conflicting)
        .await
        .expect_err("mismatched content under a used key must be rejected");
    assert_eq!(
        err.status,
        axum::http::StatusCode::CONFLICT,
        "a key reused with different content is a 409"
    );
    assert_eq!(
        catalog.list(&ctx).await.expect("list after mismatch").len(),
        1,
        "the rejected mismatch did not insert"
    );

    // Cleanup.
    uow.transaction(|| async {
        dorm_core::db::execute("DROP TABLE catalog_items", &[])
            .await
            .map_err(|e| Box::new(e) as BoxError)?;
        dorm_core::db::execute("DROP TABLE dorm_idempotency", &[])
            .await
            .map(|_| ())
            .map_err(|e| Box::new(e) as BoxError)
    })
    .await
    .expect("cleanup");
}
