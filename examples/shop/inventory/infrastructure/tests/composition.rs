//! Composition-level test for the `inventory` example: wire the
//! `InventoryService` over the **real** in-memory adapter
//! ([`InMemoryStockRepository`]) and the framework's test ports, then drive the
//! module through its public [`Inventory`] facade.

use seiva_testing::{InMemoryUnitOfWork, RecordingEventPublisher, SequenceIdGenerator};
use shop_inventory_application::InventoryService;
use shop_inventory_facade::{Inventory, ProductId, ReceiveStock, Reserve};
use shop_inventory_infrastructure::InMemoryStockRepository;

fn service() -> InventoryService<
    InMemoryStockRepository,
    SequenceIdGenerator,
    InMemoryUnitOfWork,
    RecordingEventPublisher,
> {
    InventoryService {
        repository: InMemoryStockRepository::new(),
        ids: SequenceIdGenerator::new(),
        uow: InMemoryUnitOfWork::new(),
        events: RecordingEventPublisher::new(),
    }
}

#[tokio::test]
async fn full_flow_over_the_real_in_memory_adapter() {
    let svc = service();

    svc.receive(ReceiveStock {
        product_id: 1u128.into(),
        qty: 20,
    })
    .await
    .expect("receive");
    let remaining = svc
        .reserve(Reserve {
            product_id: 1u128.into(),
            qty: 8,
        })
        .await
        .expect("reserve");
    assert_eq!(remaining.available, 12);

    assert_eq!(
        svc.available(ProductId { id: 1 })
            .await
            .expect("available")
            .available,
        12
    );

    // The adapter holds exactly one stock record, reflecting the net total.
    let items = svc.repository.items();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].available, 12);
}
