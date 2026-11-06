//! Composition-level test for the `orders` example: wire the `OrdersService`
//! over the **real** in-memory adapter ([`InMemoryOrderRepository`]) and the
//! framework's test ports, then drive the module through its public [`Orders`]
//! facade.

use seiva_testing::{InMemoryUnitOfWork, RecordingEventPublisher, SequenceIdGenerator};
use shop_orders_application::OrdersService;
use shop_orders_facade::{LineDto, OrderId, Orders, PlaceOrder};
use shop_orders_infrastructure::InMemoryOrderRepository;

fn service() -> OrdersService<
    InMemoryOrderRepository,
    SequenceIdGenerator,
    InMemoryUnitOfWork,
    RecordingEventPublisher,
> {
    OrdersService {
        repository: InMemoryOrderRepository::new(),
        ids: SequenceIdGenerator::new(),
        uow: InMemoryUnitOfWork::new(),
        events: RecordingEventPublisher::new(),
    }
}

#[tokio::test]
async fn full_flow_over_the_real_in_memory_adapter() {
    let svc = service();

    let order = svc
        .place(PlaceOrder {
            customer: "Ada".into(),
            lines: vec![
                LineDto {
                    product_id: 10,
                    qty: 2,
                },
                LineDto {
                    product_id: 11,
                    qty: 1,
                },
            ],
        })
        .await
        .expect("place");

    assert_eq!(svc.get(OrderId { id: order.id }).await.expect("get"), order);
    assert_eq!(svc.list().await.expect("list"), vec![order.clone()]);

    // The adapter actually holds the row.
    let stored = svc.repository.orders();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].lines.len(), 2);
    assert_eq!(stored[0].status, "placed");
}
