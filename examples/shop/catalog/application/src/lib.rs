//! Application layer of the `catalog` module: the `CatalogService` (which
//! implements the public facade) and the `ProductRepository` outbound port.
//!
//! Depends on `domain`, its own `facade` (DTOs + trait) and `seiva-ports`. It
//! names no `delonix_seiva::` item — the framework coupling is the facade's
//! concern (ADR 0003). The service consumes the framework's cross-cutting ports
//! (`UnitOfWork`, `IdGenerator`, `EventPublisher`) through `seiva-ports`.

#![allow(async_fn_in_trait)]

use seiva_ports::{BoxError, EventPublisher, IdGenerator, PortResult, UnitOfWork};
use shop_catalog_domain::Product;
use shop_catalog_facade::{Catalog, CatalogError, CreateProduct, ProductDto, ProductId};

/// Outbound port: persistence of products, in the domain's language.
/// Implemented in `infrastructure` (over DORM or an external library). Each
/// future is `Send` so a use case can `.await` it inside a
/// `UnitOfWork::transaction` on a multi-threaded runtime.
pub trait ProductRepository {
    fn save_product(
        &self,
        product: &Product,
    ) -> impl std::future::Future<Output = PortResult<()>> + Send;

    fn list_products(&self) -> impl std::future::Future<Output = PortResult<Vec<Product>>> + Send;

    fn get_product(
        &self,
        id: u128,
    ) -> impl std::future::Future<Output = PortResult<Option<Product>>> + Send;
}

/// The application service. It implements the [`Catalog`] facade by running each
/// use case over the ports and mapping the domain entities to DTOs.
pub struct CatalogService<R, G, U, E> {
    pub repository: R,
    pub ids: G,
    pub uow: U,
    pub events: E,
}

fn to_product_dto(p: Product) -> ProductDto {
    ProductDto {
        id: p.id.to_string(),
        name: p.name,
        price_cents: p.price_cents,
    }
}

impl<R, G, U, E> CatalogService<R, G, U, E>
where
    R: ProductRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    async fn add_a_product(&self, cmd: CreateProduct) -> PortResult<ProductDto> {
        let id = u128::from_be_bytes(self.ids.new_id().0);
        let product = Product::new(id, cmd.name, cmd.price_cents).map_err(BoxError::from)?;

        let event = serde_json::to_vec(&serde_json::json!({
            "id": product.id.to_string(),
            "name": product.name.clone(),
            "price_cents": product.price_cents,
        }))
        .map_err(BoxError::from)?;

        // One transaction per use case (ADR 0004): the save AND the event publish
        // run inside it, so with the DORM gateway the event lands in the
        // transactional outbox in the SAME transaction as the business change.
        self.uow
            .transaction(|| async {
                self.repository.save_product(&product).await?;
                self.events.publish("catalog.product_added", &event).await
            })
            .await?;

        Ok(to_product_dto(product))
    }

    async fn all_products(&self) -> PortResult<Vec<ProductDto>> {
        let products = self.repository.list_products().await?;
        Ok(products.into_iter().map(to_product_dto).collect())
    }
}

impl<R, G, U, E> Catalog for CatalogService<R, G, U, E>
where
    R: ProductRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    async fn add_product(&self, cmd: CreateProduct) -> Result<ProductDto, CatalogError> {
        // A write failure here is a domain-rule violation (empty name / zero
        // price) or an infrastructure error; both surface as `422`.
        self.add_a_product(cmd)
            .await
            .map_err(|err| CatalogError::Invalid(err.to_string()))
    }

    async fn list(&self) -> Result<Vec<ProductDto>, CatalogError> {
        self.all_products()
            .await
            .map_err(|err| CatalogError::Invalid(err.to_string()))
    }

    async fn get(&self, id: ProductId) -> Result<ProductDto, CatalogError> {
        // A missing product is a typed `NotFound` (→ `404`), distinct from a
        // repository error (→ `422`).
        let product = self
            .repository
            .get_product(id.id)
            .await
            .map_err(|err| CatalogError::Invalid(err.to_string()))?
            .ok_or_else(|| CatalogError::NotFound(format!("product {} not found", id.id)))?;
        Ok(to_product_dto(product))
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
        products: Mutex<Vec<Product>>,
    }

    impl ProductRepository for FakeRepo {
        async fn save_product(&self, product: &Product) -> PortResult<()> {
            self.products.lock().unwrap().push(product.clone());
            Ok(())
        }

        async fn list_products(&self) -> PortResult<Vec<Product>> {
            Ok(self.products.lock().unwrap().clone())
        }

        async fn get_product(&self, id: u128) -> PortResult<Option<Product>> {
            Ok(self
                .products
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.id == id)
                .cloned())
        }
    }

    fn service(
    ) -> CatalogService<FakeRepo, SequenceIdGenerator, InMemoryUnitOfWork, RecordingEventPublisher>
    {
        CatalogService {
            repository: FakeRepo::default(),
            ids: SequenceIdGenerator::new(),
            uow: InMemoryUnitOfWork::new(),
            events: RecordingEventPublisher::new(),
        }
    }

    #[tokio::test]
    async fn add_list_and_get_a_product() {
        let svc = service();

        let p = svc
            .add_product(CreateProduct {
                name: "Teapot".into(),
                price_cents: 1299,
            })
            .await
            .unwrap();
        assert_eq!((p.id.as_str(), p.price_cents), ("1", 1299));

        assert_eq!(svc.list().await.unwrap(), vec![p.clone()]);
        assert_eq!(
            svc.get(ProductId {
                id: p.id.parse().unwrap()
            })
            .await
            .unwrap(),
            p
        );

        assert_eq!(svc.uow.outcomes(), [TxOutcome::Committed]);
        assert_eq!(svc.events.topics(), ["catalog.product_added"]);
    }

    #[tokio::test]
    async fn a_zero_price_is_rejected_by_the_domain_rule() {
        let svc = service();
        let err = svc
            .add_product(CreateProduct {
                name: "Freebie".into(),
                price_cents: 0,
            })
            .await;
        assert!(err.is_err());
        assert!(svc.uow.outcomes().is_empty());
        assert_eq!(svc.events.topics(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn getting_an_unknown_product_errors() {
        let svc = service();
        assert!(svc.get(ProductId { id: 404 }).await.is_err());
    }
}
