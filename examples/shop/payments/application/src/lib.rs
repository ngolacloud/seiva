//! Application layer of the `payments` module: the `PaymentsService` (which
//! implements the public facade) and the `PaymentRepository` outbound port.
//!
//! Depends on `domain`, its own `facade` (DTOs + trait) and `seiva-ports`. It
//! names no `delonix_seiva::` item (ADR 0003) and consumes the framework's
//! cross-cutting ports (`UnitOfWork`, `IdGenerator`, `EventPublisher`) through
//! `seiva-ports`.
//!
//! The charge policy lives here: a zero amount is declined, any positive amount
//! is captured. Either way a payment is recorded and an event emitted
//! (`payments.captured` or `payments.declined`).

#![allow(async_fn_in_trait)]

use seiva_ports::{BoxError, EventPublisher, IdGenerator, PortResult, UnitOfWork};
use shop_payments_domain::Payment;
use shop_payments_facade::{Charge, PaymentDto, PaymentId, Payments, PaymentsError};

/// Outbound port: persistence of payments, in the domain's language.
/// Implemented in `infrastructure`. Each future is `Send` so a use case can
/// `.await` it inside a `UnitOfWork::transaction` on a multi-threaded runtime.
pub trait PaymentRepository {
    fn save_payment(
        &self,
        payment: &Payment,
    ) -> impl std::future::Future<Output = PortResult<()>> + Send;

    fn get_payment(
        &self,
        id: u128,
    ) -> impl std::future::Future<Output = PortResult<Option<Payment>>> + Send;
}

/// The application service. Generic over the port and the framework's
/// cross-cutting ports, exactly like the other shop modules.
pub struct PaymentsService<R, G, U, E> {
    pub repository: R,
    pub ids: G,
    pub uow: U,
    pub events: E,
}

fn to_payment_dto(p: Payment) -> PaymentDto {
    PaymentDto {
        id: p.id,
        order_id: p.order_id,
        amount_cents: p.amount_cents,
        status: p.status,
    }
}

impl<R, G, U, E> PaymentsService<R, G, U, E>
where
    R: PaymentRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    async fn charge_an_order(&self, cmd: Charge) -> PortResult<PaymentDto> {
        let id = u128::from_be_bytes(self.ids.new_id().0);

        // The demo policy: a zero amount is declined, anything else captured.
        let (payment, topic) = if cmd.amount_cents == 0 {
            (
                Payment::declined(id, cmd.order_id, cmd.amount_cents),
                "payments.declined",
            )
        } else {
            let captured =
                Payment::captured(id, cmd.order_id, cmd.amount_cents).map_err(BoxError::from)?;
            (captured, "payments.captured")
        };

        let event = serde_json::to_vec(&serde_json::json!({
            "id": payment.id.to_string(),
            "order_id": payment.order_id.to_string(),
            "amount_cents": payment.amount_cents,
            "status": payment.status.clone(),
        }))
        .map_err(BoxError::from)?;

        // One transaction per use case (ADR 0004): the save AND the event publish
        // run together, so with the DORM gateway the event lands in the outbox in
        // the SAME transaction as the business change.
        self.uow
            .transaction(|| async {
                self.repository.save_payment(&payment).await?;
                self.events.publish(topic, &event).await
            })
            .await?;

        Ok(to_payment_dto(payment))
    }
}

impl<R, G, U, E> Payments for PaymentsService<R, G, U, E>
where
    R: PaymentRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    async fn charge(&self, cmd: Charge) -> Result<PaymentDto, PaymentsError> {
        self.charge_an_order(cmd)
            .await
            .map_err(|err| PaymentsError::Invalid(err.to_string()))
    }

    async fn get(&self, id: PaymentId) -> Result<PaymentDto, PaymentsError> {
        // A missing payment is a typed `NotFound` (→ `404`), distinct from a
        // repository error (→ `422`).
        let payment = self
            .repository
            .get_payment(id.id)
            .await
            .map_err(|err| PaymentsError::Invalid(err.to_string()))?
            .ok_or_else(|| PaymentsError::NotFound(format!("payment {} not found", id.id)))?;
        Ok(to_payment_dto(payment))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seiva_testing::{
        InMemoryUnitOfWork, RecordingEventPublisher, SequenceIdGenerator, TxOutcome,
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRepo {
        payments: Mutex<Vec<Payment>>,
    }

    impl PaymentRepository for FakeRepo {
        async fn save_payment(&self, payment: &Payment) -> PortResult<()> {
            self.payments.lock().unwrap().push(payment.clone());
            Ok(())
        }

        async fn get_payment(&self, id: u128) -> PortResult<Option<Payment>> {
            Ok(self
                .payments
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.id == id)
                .cloned())
        }
    }

    fn service(
    ) -> PaymentsService<FakeRepo, SequenceIdGenerator, InMemoryUnitOfWork, RecordingEventPublisher>
    {
        PaymentsService {
            repository: FakeRepo::default(),
            ids: SequenceIdGenerator::new(),
            uow: InMemoryUnitOfWork::new(),
            events: RecordingEventPublisher::new(),
        }
    }

    #[tokio::test]
    async fn a_positive_charge_is_captured_and_read_back() {
        let svc = service();

        let p = svc
            .charge(Charge {
                order_id: 7,
                amount_cents: 2500,
            })
            .await
            .unwrap();
        assert_eq!(p.status, "captured");
        assert_eq!(svc.get(PaymentId { id: p.id }).await.unwrap(), p);

        assert_eq!(svc.uow.outcomes(), [TxOutcome::Committed]);
        assert_eq!(svc.events.topics(), ["payments.captured"]);
    }

    #[tokio::test]
    async fn a_zero_charge_is_declined_and_records_the_decline_event() {
        let svc = service();

        let p = svc
            .charge(Charge {
                order_id: 7,
                amount_cents: 0,
            })
            .await
            .unwrap();
        assert_eq!(p.status, "declined");

        // A decline is still a recorded, committed outcome.
        assert_eq!(svc.uow.outcomes(), [TxOutcome::Committed]);
        assert_eq!(svc.events.topics(), ["payments.declined"]);
    }

    #[tokio::test]
    async fn getting_an_unknown_payment_errors() {
        let svc = service();
        assert!(svc.get(PaymentId { id: 404 }).await.is_err());
    }
}
