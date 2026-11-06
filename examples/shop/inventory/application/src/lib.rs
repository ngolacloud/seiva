//! Application layer of the `inventory` module: the `InventoryService` (which
//! implements the public facade) and the `StockRepository` outbound port.
//!
//! Depends on `domain`, its own `facade` (DTOs + trait) and `seiva-ports`. It
//! names no `delonix_seiva::` item (ADR 0003) and consumes the framework's
//! cross-cutting ports (`UnitOfWork`, `EventPublisher`) through `seiva-ports`.
//! Inventory keeps its stock keyed by `product_id`, so it needs no `IdGenerator`
//! for its own entities.

#![allow(async_fn_in_trait)]

use seiva_ports::{BoxError, EventPublisher, IdGenerator, PortResult, UnitOfWork};
use shop_inventory_domain::StockItem;
use shop_inventory_facade::{
    Inventory, InventoryError, ProductId, ReceiveStock, Reserve, StockDto,
};

/// Outbound port: persistence of per-product stock, in the domain's language.
/// Implemented in `infrastructure`. Each future is `Send` so a use case can
/// `.await` it inside a `UnitOfWork::transaction` on a multi-threaded runtime.
pub trait StockRepository {
    fn get_stock(
        &self,
        product_id: u128,
    ) -> impl std::future::Future<Output = PortResult<Option<StockItem>>> + Send;

    /// Inserts or replaces the stock record for its product.
    fn save_stock(
        &self,
        stock: &StockItem,
    ) -> impl std::future::Future<Output = PortResult<()>> + Send;
}

/// The application service. Generic over the port and the framework's
/// cross-cutting ports, exactly like the other shop modules.
pub struct InventoryService<R, G, U, E> {
    pub repository: R,
    pub ids: G,
    pub uow: U,
    pub events: E,
}

fn to_stock_dto(s: StockItem) -> StockDto {
    StockDto {
        product_id: s.product_id.to_string(),
        available: s.available,
    }
}

impl<R, G, U, E> InventoryService<R, G, U, E>
where
    R: StockRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    async fn receive_stock(&self, product_id: u128, qty: u64) -> PortResult<StockDto> {
        let stock = self
            .uow
            .transaction(|| async {
                let mut stock = self
                    .repository
                    .get_stock(product_id)
                    .await?
                    .unwrap_or_else(|| StockItem::new(product_id));
                stock.receive(qty).map_err(BoxError::from)?;
                self.repository.save_stock(&stock).await?;

                let event = serde_json::to_vec(&serde_json::json!({
                    "product_id": stock.product_id.to_string(),
                    "qty": qty,
                    "available": stock.available,
                }))
                .map_err(BoxError::from)?;
                self.events.publish("inventory.received", &event).await?;

                Ok(stock)
            })
            .await?;

        Ok(to_stock_dto(stock))
    }

    async fn reserve_stock(&self, product_id: u128, qty: u64) -> PortResult<StockDto> {
        let stock = self
            .uow
            .transaction(|| async {
                let mut stock = self
                    .repository
                    .get_stock(product_id)
                    .await?
                    .ok_or_else(|| BoxError::from(format!("no stock for product {product_id}")))?;
                stock.reserve(qty).map_err(BoxError::from)?;
                self.repository.save_stock(&stock).await?;

                let event = serde_json::to_vec(&serde_json::json!({
                    "product_id": stock.product_id.to_string(),
                    "qty": qty,
                    "available": stock.available,
                }))
                .map_err(BoxError::from)?;
                self.events.publish("inventory.reserved", &event).await?;

                Ok(stock)
            })
            .await?;

        Ok(to_stock_dto(stock))
    }

    async fn available_for(&self, product_id: u128) -> PortResult<StockDto> {
        let available = self
            .repository
            .get_stock(product_id)
            .await?
            .map(|s| s.available)
            .unwrap_or(0);
        Ok(StockDto {
            product_id: product_id.to_string(),
            available,
        })
    }
}

impl<R, G, U, E> Inventory for InventoryService<R, G, U, E>
where
    R: StockRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    async fn receive(&self, cmd: ReceiveStock) -> Result<StockDto, InventoryError> {
        let product_id: u128 = cmd
            .product_id
            .parse()
            .map_err(|_| InventoryError::Invalid("product_id must be an integer".to_string()))?;
        self.receive_stock(product_id, cmd.qty)
            .await
            .map_err(|err| InventoryError::Invalid(err.to_string()))
    }

    async fn reserve(&self, cmd: Reserve) -> Result<StockDto, InventoryError> {
        let product_id: u128 = cmd
            .product_id
            .parse()
            .map_err(|_| InventoryError::Invalid("product_id must be an integer".to_string()))?;
        self.reserve_stock(product_id, cmd.qty)
            .await
            .map_err(|err| InventoryError::Invalid(err.to_string()))
    }

    async fn available(&self, q: ProductId) -> Result<StockDto, InventoryError> {
        self.available_for(q.id)
            .await
            .map_err(|err| InventoryError::Invalid(err.to_string()))
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
        stock: Mutex<Vec<StockItem>>,
    }

    impl StockRepository for FakeRepo {
        async fn get_stock(&self, product_id: u128) -> PortResult<Option<StockItem>> {
            Ok(self
                .stock
                .lock()
                .unwrap()
                .iter()
                .find(|s| s.product_id == product_id)
                .cloned())
        }

        async fn save_stock(&self, stock: &StockItem) -> PortResult<()> {
            let mut all = self.stock.lock().unwrap();
            match all.iter_mut().find(|s| s.product_id == stock.product_id) {
                Some(existing) => *existing = stock.clone(),
                None => all.push(stock.clone()),
            }
            Ok(())
        }
    }

    fn service(
    ) -> InventoryService<FakeRepo, SequenceIdGenerator, InMemoryUnitOfWork, RecordingEventPublisher>
    {
        InventoryService {
            repository: FakeRepo::default(),
            ids: SequenceIdGenerator::new(),
            uow: InMemoryUnitOfWork::new(),
            events: RecordingEventPublisher::new(),
        }
    }

    #[tokio::test]
    async fn receive_then_reserve_tracks_available() {
        let svc = service();

        let after_receipt = svc
            .receive(ReceiveStock {
                product_id: "42".into(),
                qty: 10,
            })
            .await
            .unwrap();
        assert_eq!(after_receipt.available, 10);

        let after_reserve = svc
            .reserve(Reserve {
                product_id: "42".into(),
                qty: 4,
            })
            .await
            .unwrap();
        assert_eq!(after_reserve.available, 6);

        assert_eq!(
            svc.available(ProductId { id: 42 }).await.unwrap().available,
            6
        );

        assert_eq!(
            svc.uow.outcomes(),
            [TxOutcome::Committed, TxOutcome::Committed]
        );
        assert_eq!(
            svc.events.topics(),
            ["inventory.received", "inventory.reserved"]
        );
    }

    #[tokio::test]
    async fn reserving_more_than_available_errors_and_commits_nothing() {
        let svc = service();
        svc.receive(ReceiveStock {
            product_id: "42".into(),
            qty: 2,
        })
        .await
        .unwrap();

        let err = svc
            .reserve(Reserve {
                product_id: "42".into(),
                qty: 5,
            })
            .await;
        assert!(err.is_err());

        // The receipt committed; the over-reservation rolled back.
        assert_eq!(
            svc.uow.outcomes(),
            [TxOutcome::Committed, TxOutcome::RolledBack]
        );
        assert_eq!(svc.events.topics(), ["inventory.received"]);
        // The stock is intact after the failed reservation.
        assert_eq!(
            svc.available(ProductId { id: 42 }).await.unwrap().available,
            2
        );
    }

    #[tokio::test]
    async fn available_is_zero_for_an_unstocked_product() {
        let svc = service();
        assert_eq!(
            svc.available(ProductId { id: 7 }).await.unwrap().available,
            0
        );
    }
}
