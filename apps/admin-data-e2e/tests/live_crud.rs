//! Live proof that the `Catalog` module's five CRUD operations work end to end
//! over a real PostgreSQL through the DORM gateway (ADR 0009 Phase-3). Gated on
//! `SEIVA_DORM_TEST_DSN`; skips cleanly when unset.
//!
//! It drives the `CatalogController` operations directly (the same operations the
//! admin console calls over HTTP), proving create → list → get → update → delete
//! each persist through DORM.

use std::sync::Arc;

use admin_data_e2e::{
    catalog_table_ddl, Catalog, CatalogController, CreateItem, DeleteItem, UpdateItem,
};
use seiva_data_dorm::{connect, DormConfig, DormUnitOfWork};
use seiva_ports::{BoxError, UnitOfWork};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_catalog_full_crud_round_trip() {
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

    // Fresh table.
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

    let catalog = CatalogController::new(Arc::clone(&uow));

    // CREATE.
    let created = catalog
        .create(CreateItem {
            name: "widget".into(),
            price: 100,
        })
        .await
        .expect("create");
    assert_eq!(created.name, "widget");
    assert_eq!(created.price, 100);
    let id = created.id;

    // GET.
    let fetched = catalog.get(id).await.expect("get");
    assert_eq!(fetched, created);

    // A second create, then LIST returns both, ordered by id.
    let second = catalog
        .create(CreateItem {
            name: "gadget".into(),
            price: 250,
        })
        .await
        .expect("create 2");
    let list = catalog.list().await.expect("list");
    assert_eq!(list.len(), 2, "two items listed");
    assert_eq!(list[0].id, id);
    assert_eq!(list[1].id, second.id);

    // UPDATE the first item.
    let updated = catalog
        .update(UpdateItem {
            id,
            name: "widget-mk2".into(),
            price: 150,
        })
        .await
        .expect("update");
    assert_eq!(updated.name, "widget-mk2");
    assert_eq!(updated.price, 150);
    assert_eq!(
        catalog.get(id).await.expect("get after update").name,
        "widget-mk2"
    );

    // DELETE the first item; it is gone, the second remains.
    catalog.delete(DeleteItem { id }).await.expect("delete");
    assert!(
        catalog.get(id).await.is_err(),
        "deleted item must not be found"
    );
    let remaining = catalog.list().await.expect("list after delete");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, second.id);

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
