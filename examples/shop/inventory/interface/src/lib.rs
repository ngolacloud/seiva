//! Interface layer of the `inventory` module: the HTTP transport.
//!
//! The `#[seiva::controller]` lives HERE; the controller [`InventoryController<R>`]
//! is generic over the repository port `R`, so this layer depends only on
//! `application` and never on `infrastructure`. The composition root injects the
//! concrete repository and merges `inventory_router(controller)`.
//!
//! # Generated routes
//!
//! | facade method | route                              | success | required permission |
//! |---------------|------------------------------------|---------|---------------------|
//! | `receive`     | `POST /v1/inventories/receive`      | `200`   | `inventory.manage`  |
//! | `reserve`     | `POST /v1/inventories/reserve`      | `200`   | `inventory.manage`  |
//! | `available`   | `POST /v1/inventories/available`    | `200`   | `inventory.read`    |

#![forbid(unsafe_code)]

use std::sync::Arc;

use delonix_seiva::admin::{AdminPermissions, AdminResource};
use delonix_seiva::{controller, register_admin};

use shop_inventory_application::{InventoryService, StockRepository};
use shop_inventory_facade::{
    Inventory, InventoryError, InventoryFacade, ProductId, ReceiveStock, Reserve, StockDto,
};

register_admin!(AdminResource {
    namespace: "inventory",
    resource: "inventories",
    label: "Inventory",
    facade: "Inventory",
    version: "v1",
    list_fields: &["product_id", "available"],
    search_fields: &[],
    filters: &["product_id"],
    ordering: &["product_id"],
    page_size: 25,
    permissions: AdminPermissions {
        view: "inventory.read",
        create: "inventory.manage",
        change: "",
        delete: "",
    },
});

use runtime::{DirectUnitOfWork, NoopEventPublisher, SystemIds};

/// In-process ports that let the interface compose a runnable `inventory` service
/// without the DORM data layer.
mod runtime {
    use async_trait::async_trait;
    use seiva_ports::{EventPublisher, Id, IdGenerator, PortResult, UnitOfWork};
    use std::future::Future;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub struct SystemIds;

    impl IdGenerator for SystemIds {
        fn new_id(&self) -> Id {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            Id(nanos.to_be_bytes())
        }
    }

    pub struct DirectUnitOfWork;

    impl UnitOfWork for DirectUnitOfWork {
        fn transaction<'a, T, F, Fut>(
            &'a self,
            work: F,
        ) -> std::pin::Pin<Box<dyn Future<Output = PortResult<T>> + Send + 'a>>
        where
            F: FnOnce() -> Fut + Send + 'a,
            Fut: Future<Output = PortResult<T>> + Send + 'a,
            T: Send + 'a,
        {
            Box::pin(async move { work().await })
        }
    }

    pub struct NoopEventPublisher;

    #[async_trait]
    impl EventPublisher for NoopEventPublisher {
        async fn publish(&self, _topic: &str, _payload: &[u8]) -> PortResult<()> {
            Ok(())
        }
    }
}

/// The concrete `Inventory` service this controller wraps.
type Service<R> = InventoryService<R, SystemIds, DirectUnitOfWork, NoopEventPublisher>;

/// Transport adapter: a concrete `Inventory` service exposed over HTTP, generic
/// over the repository port `R`.
pub struct InventoryController<R>(Arc<Service<R>>);

impl<R> Clone for InventoryController<R> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<R: StockRepository> InventoryController<R> {
    /// Builds the controller over `repository` and the in-process ports.
    pub fn new(repository: R) -> Self {
        InventoryController(Arc::new(InventoryService {
            repository,
            ids: SystemIds,
            uow: DirectUnitOfWork,
            events: NoopEventPublisher,
        }))
    }
}

#[controller]
impl<R> Inventory for InventoryController<R>
where
    R: StockRepository + Send + Sync + 'static,
{
    async fn receive(&self, cmd: ReceiveStock) -> Result<StockDto, InventoryError> {
        self.0.receive(cmd).await
    }

    async fn reserve(&self, cmd: Reserve) -> Result<StockDto, InventoryError> {
        self.0.reserve(cmd).await
    }

    async fn available(&self, q: ProductId) -> Result<StockDto, InventoryError> {
        self.0.available(q).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use seiva_http::{Principal, PROBLEM_JSON};
    use serde_json::Value;
    use shop_inventory_infrastructure::InMemoryStockRepository;
    use tower::ServiceExt;

    async fn json_body(body: Body) -> Value {
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn demo_router() -> axum::Router {
        inventory_router(InventoryController::new(InMemoryStockRepository::new()))
    }

    #[tokio::test]
    async fn receive_is_forbidden_without_permission() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/inventories/receive")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"product_id":42,"qty":10}"#))
            .unwrap();

        let response = demo_router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = json_body(response.into_body()).await;
        assert!(body["detail"]
            .as_str()
            .unwrap()
            .contains("inventory.manage"));
    }

    #[tokio::test]
    async fn receive_zero_qty_is_422_problem() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/inventories/receive")
            .header("content-type", "application/json")
            .extension(Principal::new("mgr", ["inventory.manage"]))
            .body(Body::from(r#"{"product_id":42,"qty":0}"#))
            .unwrap();

        let response = demo_router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.headers()["content-type"], PROBLEM_JSON);
    }

    #[tokio::test]
    async fn receive_then_reserve_then_available_round_trip() {
        let app = demo_router();

        let receive = Request::builder()
            .method("POST")
            .uri("/v1/inventories/receive")
            .header("content-type", "application/json")
            .extension(Principal::new("mgr", ["inventory.manage"]))
            .body(Body::from(r#"{"product_id":42,"qty":10}"#))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(receive).await.unwrap().status(),
            StatusCode::OK
        );

        let reserve = Request::builder()
            .method("POST")
            .uri("/v1/inventories/reserve")
            .header("content-type", "application/json")
            .extension(Principal::new("mgr", ["inventory.manage"]))
            .body(Body::from(r#"{"product_id":42,"qty":4}"#))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(reserve).await.unwrap().status(),
            StatusCode::OK
        );

        let available = Request::builder()
            .method("POST")
            .uri("/v1/inventories/available")
            .header("content-type", "application/json")
            .extension(Principal::new("clerk", ["inventory.read"]))
            .body(Body::from(r#"{"id":42}"#))
            .unwrap();
        let response = app.oneshot(available).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response.into_body()).await;
        assert_eq!(body["available"], 6);
    }
}
