//! Composition-level test for the `catalog` example: wire the `CatalogService`
//! over the **real** in-memory infrastructure adapter
//! ([`InMemoryProductRepository`]) and the framework's test ports, then drive
//! the whole module through its public [`Catalog`] facade — the same shape a
//! composition root in `apps/` assembles.

use seiva_testing::{InMemoryUnitOfWork, RecordingEventPublisher, SequenceIdGenerator};
use shop_catalog_application::CatalogService;
use shop_catalog_facade::{Catalog, CreateProduct, ProductId};
use shop_catalog_infrastructure::InMemoryProductRepository;

fn service() -> CatalogService<
    InMemoryProductRepository,
    SequenceIdGenerator,
    InMemoryUnitOfWork,
    RecordingEventPublisher,
> {
    CatalogService {
        repository: InMemoryProductRepository::new(),
        ids: SequenceIdGenerator::new(),
        uow: InMemoryUnitOfWork::new(),
        events: RecordingEventPublisher::new(),
    }
}

#[tokio::test]
async fn full_flow_over_the_real_in_memory_adapter() {
    let svc = service();

    let teapot = svc
        .add_product(CreateProduct {
            name: "Teapot".into(),
            price_cents: 1299,
        })
        .await
        .expect("add teapot");
    svc.add_product(CreateProduct {
        name: "Mug".into(),
        price_cents: 499,
    })
    .await
    .expect("add mug");

    // The index view reads back the real stored state.
    assert_eq!(svc.list().await.expect("list").len(), 2);
    assert_eq!(
        svc.get(ProductId { id: teapot.id })
            .await
            .expect("get teapot"),
        teapot
    );

    // The adapter actually holds the rows (its own composition-level accessor).
    assert_eq!(svc.repository.products().len(), 2);
}
