//! Application layer of the `orders` module: the `OrdersService` (which
//! implements the public facade) and the `OrderRepository` outbound port.
//!
//! Depends on `domain`, its own `facade` (DTOs + trait) and `seiva-ports`. It
//! names no `delonix_seiva::` item (ADR 0003) and consumes the framework's
//! cross-cutting ports (`UnitOfWork`, `IdGenerator`, `EventPublisher`) through
//! `seiva-ports`.

#![allow(async_fn_in_trait)]

use seiva_ports::{EventPublisher, IdGenerator, Scope, UnitOfWork};
use shop_orders_domain::{Line, Order};
// Classic DDD (ADR 0003): the repository PORT is defined in the DOMAIN. Re-exported
// here so `interface`/`cqrs` import it from `application` (never naming `domain`),
// keeping the layer rule intact; the application maps its domain error to `PortResult`.
pub use shop_orders_domain::{OrderRepository, RepositoryError};
use shop_orders_facade::{LineDto, OrderDto, OrderId, OrdersError, PlaceOrder};

pub mod cqrs;

/// The application service. Generic over the port and the framework's
/// cross-cutting ports, exactly like the other shop modules.
pub struct OrdersService<R, G, U, E> {
    pub repository: R,
    pub ids: G,
    pub uow: U,
    pub events: E,
}

fn to_line_dto(l: &Line) -> LineDto {
    LineDto {
        product_id: l.product_id.into(),
        qty: l.qty,
    }
}

fn to_order_dto(o: Order) -> OrderDto {
    OrderDto {
        id: o.id.into(),
        customer: o.customer,
        lines: o.lines.iter().map(to_line_dto).collect(),
        status: o.status,
    }
}

impl<R, G, U, E> OrdersService<R, G, U, E>
where
    R: OrderRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    /// The side-effect-free part of placing an order: mint the id, build the domain
    /// [`Order`] (enforcing its invariants) and serialise the `orders.placed` event
    /// payload. No port is touched and no transaction is opened, so a failure here
    /// is a pure validation/domain error that can be raised *before* any
    /// transactional scope — which is what lets both the direct [`place`](Self::place)
    /// and the bus's `Transactional` keep an invalid request from ever opening a
    /// transaction.
    fn prepare(&self, cmd: PlaceOrder) -> Result<(Order, Vec<u8>), OrdersError> {
        let id = u128::from_be_bytes(self.ids.new_id().0);
        let lines: Vec<Line> = cmd
            .lines
            .into_iter()
            .map(|l| Line {
                product_id: l.product_id.get(),
                qty: l.qty,
            })
            .collect();
        let order =
            Order::new(id, cmd.customer, lines).map_err(|e| OrdersError::Invalid(e.to_string()))?;

        let event = serde_json::to_vec(&serde_json::json!({
            "id": order.id.to_string(),
            "customer": order.customer.clone(),
            "lines": order
                .lines
                .iter()
                .map(|l| serde_json::json!({
                    "product_id": l.product_id.to_string(),
                    "qty": l.qty,
                }))
                .collect::<Vec<_>>(),
        }))
        .map_err(|e| OrdersError::Invalid(e.to_string()))?;

        Ok((order, event))
    }

    /// The use case body **without** its own transaction: persist the order and
    /// publish `orders.placed`.
    ///
    /// This is the transaction-free core so it can be nested as the innermost link
    /// of the ADR 0020 command bus chain `Validate → Transactional{ Idempotent →
    /// handler }`, where the enclosing `Transactional` owns the transaction and the
    /// inner `Idempotent` the dedup — the save, the publish and the key reservation
    /// must all commit as one atom, so this must not open a nested transaction of
    /// its own. The in-process [`place`](Self::place) wraps the same work in a
    /// transaction for the direct (non-bus) caller.
    ///
    /// The service stays transport-free: it names no `seiva-http` item and never
    /// sees a `Ctx`.
    pub async fn place_work(&self, cmd: PlaceOrder) -> Result<OrderDto, OrdersError> {
        let (order, event) = self.prepare(cmd)?;
        // Save AND publish together (ADR 0004/0021): with the DORM gateway the event
        // lands in the outbox in the SAME transaction as the business change — but
        // the transaction itself is opened by the caller (`place` or the bus's
        // `Transactional`), not here.
        self.repository
            .save_order(&order)
            .await
            .map_err(|e| OrdersError::Invalid(e.to_string()))?;
        self.events
            .publish("orders.placed", &event)
            .await
            .map_err(|e| OrdersError::Invalid(e.to_string()))?;
        Ok(to_order_dto(order))
    }

    /// Place an order in one transaction (the direct, non-bus path). Validates and
    /// builds the order *before* opening the transaction (so an invalid request
    /// opens none), then runs the save and publish inside a [`UnitOfWork`]
    /// transaction, committing on `Ok` and rolling back on `Err` — the behaviour the
    /// module had before the bus cutover, kept for direct/unit use.
    pub async fn place(&self, cmd: PlaceOrder) -> Result<OrderDto, OrdersError> {
        let (order, event) = self.prepare(cmd)?;
        self.uow
            .transaction(Scope::Global, || async {
                self.repository.save_order(&order).await?;
                self.events.publish("orders.placed", &event).await
            })
            .await
            .map_err(|err| OrdersError::Invalid(err.to_string()))?;
        Ok(to_order_dto(order))
    }

    /// Fetch one order by id. A missing order is a typed `NotFound` (→ `404`),
    /// distinct from a repository error (→ `422`).
    pub async fn get(&self, id: OrderId) -> Result<OrderDto, OrdersError> {
        let order = self
            .repository
            .get_order(id.id)
            .await
            .map_err(|err| OrdersError::Invalid(err.to_string()))?
            .ok_or_else(|| OrdersError::NotFound(format!("order {} not found", id.id)))?;
        Ok(to_order_dto(order))
    }

    /// Every order placed so far.
    pub async fn list(&self) -> Result<Vec<OrderDto>, OrdersError> {
        let orders = self
            .repository
            .list_orders()
            .await
            .map_err(|err| OrdersError::Invalid(err.to_string()))?;
        Ok(orders.into_iter().map(to_order_dto).collect())
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
        orders: Mutex<Vec<Order>>,
    }

    impl OrderRepository for FakeRepo {
        async fn save_order(&self, order: &Order) -> Result<(), RepositoryError> {
            self.orders.lock().unwrap().push(order.clone());
            Ok(())
        }

        async fn get_order(&self, id: u128) -> Result<Option<Order>, RepositoryError> {
            Ok(self
                .orders
                .lock()
                .unwrap()
                .iter()
                .find(|o| o.id == id)
                .cloned())
        }

        async fn list_orders(&self) -> Result<Vec<Order>, RepositoryError> {
            Ok(self.orders.lock().unwrap().clone())
        }
    }

    fn service(
    ) -> OrdersService<FakeRepo, SequenceIdGenerator, InMemoryUnitOfWork, RecordingEventPublisher>
    {
        OrdersService {
            repository: FakeRepo::default(),
            ids: SequenceIdGenerator::new(),
            uow: InMemoryUnitOfWork::new(),
            events: RecordingEventPublisher::new(),
        }
    }

    fn line(product_id: u128, qty: u64) -> LineDto {
        LineDto {
            product_id: product_id.into(),
            qty,
        }
    }

    #[tokio::test]
    async fn place_get_and_list_an_order() {
        let svc = service();

        let o = svc
            .place(PlaceOrder {
                customer: "Ada".into(),
                lines: vec![line(10, 2), line(11, 1)],
            })
            .await
            .unwrap();
        assert_eq!(o.id.to_string(), "1");
        assert_eq!(o.status, "placed");
        assert_eq!(o.lines.len(), 2);

        assert_eq!(svc.get(OrderId { id: o.id.get() }).await.unwrap(), o);
        assert_eq!(svc.list().await.unwrap(), vec![o]);

        assert_eq!(svc.uow.outcomes(), [TxOutcome::Committed]);
        assert_eq!(svc.events.topics(), ["orders.placed"]);
    }

    #[tokio::test]
    async fn an_order_with_no_lines_is_rejected_by_the_domain_rule() {
        let svc = service();
        let err = svc
            .place(PlaceOrder {
                customer: "Ada".into(),
                lines: vec![],
            })
            .await;
        assert!(err.is_err());
        assert!(svc.uow.outcomes().is_empty());
        assert_eq!(svc.events.topics(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn getting_an_unknown_order_errors() {
        let svc = service();
        assert!(svc.get(OrderId { id: 404 }).await.is_err());
    }
}
