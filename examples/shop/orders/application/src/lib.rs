//! Application layer of the `orders` module: the `OrdersService` (which
//! implements the public facade) and the `OrderRepository` outbound port.
//!
//! Depends on `domain`, its own `facade` (DTOs + trait) and `seiva-ports`. It
//! names no `delonix_seiva::` item (ADR 0003) and consumes the framework's
//! cross-cutting ports (`UnitOfWork`, `IdGenerator`, `EventPublisher`) through
//! `seiva-ports`.

#![allow(async_fn_in_trait)]

use seiva_ports::{BoxError, EventPublisher, IdGenerator, PortResult, UnitOfWork};
use shop_orders_domain::{Line, Order};
use shop_orders_facade::{LineDto, OrderDto, OrderId, Orders, OrdersError, PlaceOrder};

pub mod cqrs;

/// Outbound port: persistence of orders, in the domain's language. Implemented
/// in `infrastructure`. Each future is `Send` so a use case can `.await` it
/// inside a `UnitOfWork::transaction` on a multi-threaded runtime.
pub trait OrderRepository {
    fn save_order(&self, order: &Order)
        -> impl std::future::Future<Output = PortResult<()>> + Send;

    fn get_order(
        &self,
        id: u128,
    ) -> impl std::future::Future<Output = PortResult<Option<Order>>> + Send;

    fn list_orders(&self) -> impl std::future::Future<Output = PortResult<Vec<Order>>> + Send;
}

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
        product_id: l.product_id.to_string(),
        qty: l.qty,
    }
}

fn to_order_dto(o: Order) -> OrderDto {
    OrderDto {
        id: o.id.to_string(),
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
    async fn place_an_order(&self, cmd: PlaceOrder) -> PortResult<OrderDto> {
        let id = u128::from_be_bytes(self.ids.new_id().0);
        let lines = cmd
            .lines
            .into_iter()
            .map(|l| {
                let product_id: u128 = l
                    .product_id
                    .parse()
                    .map_err(|_| BoxError::from("product_id must be an integer"))?;
                Ok(Line {
                    product_id,
                    qty: l.qty,
                })
            })
            .collect::<PortResult<Vec<Line>>>()?;
        let order = Order::new(id, cmd.customer, lines).map_err(BoxError::from)?;

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
        .map_err(BoxError::from)?;

        // One transaction per use case (ADR 0004): save AND event publish run
        // together, so with the DORM gateway the event lands in the outbox in the
        // SAME transaction as the business change.
        self.uow
            .transaction(|| async {
                self.repository.save_order(&order).await?;
                self.events.publish("orders.placed", &event).await
            })
            .await?;

        Ok(to_order_dto(order))
    }

    async fn all_orders(&self) -> PortResult<Vec<OrderDto>> {
        let orders = self.repository.list_orders().await?;
        Ok(orders.into_iter().map(to_order_dto).collect())
    }
}

impl<R, G, U, E> Orders for OrdersService<R, G, U, E>
where
    R: OrderRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    async fn place(&self, cmd: PlaceOrder) -> Result<OrderDto, OrdersError> {
        self.place_an_order(cmd)
            .await
            .map_err(|err| OrdersError::Invalid(err.to_string()))
    }

    async fn get(&self, id: OrderId) -> Result<OrderDto, OrdersError> {
        // A missing order is a typed `NotFound` (→ `404`), distinct from a
        // repository error (→ `422`).
        let order = self
            .repository
            .get_order(id.id)
            .await
            .map_err(|err| OrdersError::Invalid(err.to_string()))?
            .ok_or_else(|| OrdersError::NotFound(format!("order {} not found", id.id)))?;
        Ok(to_order_dto(order))
    }

    async fn list(&self) -> Result<Vec<OrderDto>, OrdersError> {
        self.all_orders()
            .await
            .map_err(|err| OrdersError::Invalid(err.to_string()))
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
        async fn save_order(&self, order: &Order) -> PortResult<()> {
            self.orders.lock().unwrap().push(order.clone());
            Ok(())
        }

        async fn get_order(&self, id: u128) -> PortResult<Option<Order>> {
            Ok(self
                .orders
                .lock()
                .unwrap()
                .iter()
                .find(|o| o.id == id)
                .cloned())
        }

        async fn list_orders(&self) -> PortResult<Vec<Order>> {
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

    fn line(product_id: &str, qty: u64) -> LineDto {
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
                lines: vec![line("10", 2), line("11", 1)],
            })
            .await
            .unwrap();
        assert_eq!(o.id, "1");
        assert_eq!(o.status, "placed");
        assert_eq!(o.lines.len(), 2);

        assert_eq!(
            svc.get(OrderId {
                id: o.id.parse().unwrap()
            })
            .await
            .unwrap(),
            o
        );
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
