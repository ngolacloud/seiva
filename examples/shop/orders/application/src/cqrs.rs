//! CQRS surface for the `orders` module.
//!
//! A small command/query mediator built on `seiva-cqrs`, living in the
//! application layer (where CQRS belongs — `seiva-cqrs` is allowed here, the
//! framework runtime `delonix-seiva` is not). It demonstrates the hexagonal
//! read/write split the shop tutorial teaches:
//!
//! - [`PlaceOrderCommand`] is a write ([`Command`]) carried out by
//!   [`OrdersHandler`] over the module's own [`OrdersService`].
//! - [`GetOrder`] is a read ([`Query`]) served by the same handler; wrapped in
//!   `CacheQueries` it is memoised, so a repeated read for the same order id hits
//!   the cache instead of the repository.
//!
//! Both are dispatched through one [`Mediator`](seiva_cqrs::Mediator) so the
//! caller depends on the mediator, never on the concrete handler.

use std::sync::Arc;

use seiva_cqrs::{Command, CommandHandler, Query, QueryHandler};
use seiva_ports::{BoxError, EventPublisher, IdGenerator, PortResult, UnitOfWork};
use shop_orders_facade::{OrderDto, OrderId, Orders, PlaceOrder};

use crate::{OrderRepository, OrdersService};

/// Write message: place an order. Its output is the placed [`OrderDto`].
pub struct PlaceOrderCommand(pub PlaceOrder);

impl Command for PlaceOrderCommand {
    type Output = OrderDto;
}

/// Read message: fetch one order by id. `Eq + Hash + Clone` so it can be its own
/// cache key under `CacheQueries`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GetOrder {
    /// The order's id.
    pub id: u128,
}

impl Query for GetOrder {
    type Output = OrderDto;
}

/// The use case that carries out both messages, over the module's own
/// [`OrdersService`]. Held behind an [`Arc`] so one handler serves the command
/// and query sides of the mediator.
pub struct OrdersHandler<R, G, U, E> {
    service: Arc<OrdersService<R, G, U, E>>,
}

impl<R, G, U, E> OrdersHandler<R, G, U, E> {
    /// Wraps a ready `OrdersService`.
    pub fn new(service: Arc<OrdersService<R, G, U, E>>) -> Self {
        Self { service }
    }
}

impl<R, G, U, E> CommandHandler<PlaceOrderCommand> for OrdersHandler<R, G, U, E>
where
    R: OrderRepository + Send + Sync,
    G: IdGenerator + Send + Sync,
    U: UnitOfWork + Send + Sync,
    E: EventPublisher + Send + Sync,
{
    async fn handle(&self, cmd: PlaceOrderCommand) -> PortResult<OrderDto> {
        self.service
            .place(cmd.0)
            .await
            .map_err(|err| BoxError::from(err.to_string()))
    }
}

impl<R, G, U, E> QueryHandler<GetOrder> for OrdersHandler<R, G, U, E>
where
    R: OrderRepository + Send + Sync,
    G: IdGenerator + Send + Sync,
    U: UnitOfWork + Send + Sync,
    E: EventPublisher + Send + Sync,
{
    async fn handle(&self, query: GetOrder) -> PortResult<OrderDto> {
        self.service
            .get(OrderId { id: query.id })
            .await
            .map_err(|err| BoxError::from(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seiva_cqrs::{CacheQueries, Instrument, Mediator};
    use seiva_ports::PortResult;
    use seiva_testing::{InMemoryUnitOfWork, RecordingEventPublisher, SequenceIdGenerator};
    use shop_orders_domain::Order;
    use shop_orders_facade::LineDto;
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

    #[tokio::test]
    async fn command_places_an_order_and_query_reads_it_back_through_the_mediator() {
        let handler = OrdersHandler::new(Arc::new(service()));
        // Instrument (counts every dispatch) over CacheQueries (memoises the
        // read) over the use-case handler — the inside-out composition.
        let mediator = Mediator::new(Instrument::new(CacheQueries::<_, GetOrder>::new(handler)));

        // Write: dispatch the command.
        let placed = mediator
            .send(PlaceOrderCommand(PlaceOrder {
                customer: "Ada".into(),
                lines: vec![LineDto {
                    product_id: 10u128.into(),
                    qty: 2,
                }],
            }))
            .await
            .expect("place order");
        assert_eq!(placed.status, "placed");
        assert_eq!(placed.id.to_string(), "1");

        // Read: query it back. First call misses the cache (hits the repository),
        // the second is served from the cache.
        let order_id: u128 = placed.id.get();
        let first = mediator.query(GetOrder { id: order_id }).await.unwrap();
        let second = mediator.query(GetOrder { id: order_id }).await.unwrap();
        assert_eq!(first, placed);
        assert_eq!(second, first);

        // One command + two queries were dispatched; the query cache holds the
        // one order id.
        assert_eq!(mediator.handler().dispatched(), 3);
        assert_eq!(mediator.handler().inner().len(), 1);
    }

    #[tokio::test]
    async fn an_invalid_command_fails_through_the_mediator() {
        let handler = OrdersHandler::new(Arc::new(service()));
        let mediator = Mediator::new(CacheQueries::<_, GetOrder>::new(handler));

        // An order with no lines violates a domain rule.
        let result = mediator
            .send(PlaceOrderCommand(PlaceOrder {
                customer: "Ada".into(),
                lines: vec![],
            }))
            .await;
        assert!(result.is_err());
    }
}
