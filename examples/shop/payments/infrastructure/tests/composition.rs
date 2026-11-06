//! Composition-level test for the `payments` example: wire the `PaymentsService`
//! over the **real** in-memory adapter ([`InMemoryPaymentRepository`]) and the
//! framework's test ports, then drive the module through its public [`Payments`]
//! facade.

use seiva_testing::{InMemoryUnitOfWork, RecordingEventPublisher, SequenceIdGenerator};
use shop_payments_application::PaymentsService;
use shop_payments_facade::{Charge, PaymentId, Payments};
use shop_payments_infrastructure::InMemoryPaymentRepository;

fn service() -> PaymentsService<
    InMemoryPaymentRepository,
    SequenceIdGenerator,
    InMemoryUnitOfWork,
    RecordingEventPublisher,
> {
    PaymentsService {
        repository: InMemoryPaymentRepository::new(),
        ids: SequenceIdGenerator::new(),
        uow: InMemoryUnitOfWork::new(),
        events: RecordingEventPublisher::new(),
    }
}

#[tokio::test]
async fn full_flow_over_the_real_in_memory_adapter() {
    let svc = service();

    let captured = svc
        .charge(Charge {
            order_id: 7,
            amount_cents: 2500,
        })
        .await
        .expect("capture");
    assert_eq!(captured.status, "captured");

    let declined = svc
        .charge(Charge {
            order_id: 8,
            amount_cents: 0,
        })
        .await
        .expect("decline");
    assert_eq!(declined.status, "declined");

    // Both outcomes read back from the real adapter.
    assert_eq!(
        svc.get(PaymentId { id: captured.id })
            .await
            .expect("get captured"),
        captured
    );
    assert_eq!(svc.repository.payments().len(), 2);
}
