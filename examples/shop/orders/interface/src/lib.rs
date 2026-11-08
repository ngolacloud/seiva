//! Interface layer of the `orders` module: the HTTP transport.
//!
//! The `#[seiva::controller]` lives HERE; the controller [`OrdersController<R>`]
//! is generic over the repository port `R`, so this layer depends only on
//! `application` and never on `infrastructure`. The composition root injects the
//! concrete repository and merges `orders_router(controller)`.
//!
//! # Generated routes
//!
//! | facade method | route                     | success | required permission |
//! |---------------|---------------------------|---------|---------------------|
//! | `place`       | `POST /v1/orders/place`   | `200`   | `orders.manage`     |
//! | `get`         | `GET  /v1/orders/{id}`    | `200`   | `orders.read`       |
//! | `list`        | `GET  /v1/orders`         | `200`   | `orders.read`       |

#![forbid(unsafe_code)]

use std::sync::Arc;

use delonix_seiva::admin::{AdminPermissions, AdminResource};
use delonix_seiva::{controller, register_admin};
use seiva_ports::EventPublisher;

use shop_orders_application::{OrderRepository, OrdersService};
use shop_orders_facade::{OrderDto, OrderId, Orders, OrdersError, OrdersFacade, PlaceOrder};

register_admin!(AdminResource {
    namespace: "orders",
    resource: "orders",
    label: "Orders",
    facade: "Orders",
    version: "v1",
    list_fields: &["id", "customer", "status"],
    search_fields: &["customer"],
    filters: &["status"],
    ordering: &["id"],
    page_size: 25,
    permissions: AdminPermissions {
        view: "orders.read",
        create: "orders.manage",
        change: "",
        delete: "",
    },
});

use runtime::{DirectUnitOfWork, NoopEventPublisher, SystemIds};

/// In-process ports that let the interface compose a runnable `orders` service
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

/// The concrete `Orders` service this controller wraps, generic over the event
/// publisher port `E` so the composition root can swap the default no-op for a real
/// one (an outbox) without touching the module.
type Service<R, E> = OrdersService<R, SystemIds, DirectUnitOfWork, E>;

/// Transport adapter: a concrete `Orders` service exposed over HTTP, generic over
/// the repository port `R` and the event publisher port `E`.
///
/// `E` defaults to `NoopEventPublisher`, so [`new`](Self::new) stays a one-arg
/// constructor for the standalone demo where the `orders.placed` event is swallowed.
/// The composition root that wants the event to actually flow (ADR 0021) injects a
/// real publisher through [`with_events`](Self::with_events) — e.g. an in-memory
/// outbox the in-process dispatcher drains.
pub struct OrdersController<R, E = NoopEventPublisher>(Arc<Service<R, E>>);

impl<R, E> Clone for OrdersController<R, E> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<R: OrderRepository> OrdersController<R, NoopEventPublisher> {
    /// Builds the controller over `repository` and the in-process ports, with the
    /// `orders.placed` event swallowed by a `NoopEventPublisher`. The runnable
    /// default for the standalone module demo.
    pub fn new(repository: R) -> Self {
        Self::with_events(repository, NoopEventPublisher)
    }
}

impl<R: OrderRepository, E: EventPublisher> OrdersController<R, E> {
    /// Builds the controller over `repository` and an injected event publisher
    /// `events`, so a published `orders.placed` lands wherever `events` writes it
    /// (an outbox the composition root then dispatches). The id generator and the
    /// commit-on-`Ok` unit of work stay the in-process defaults — the use-case body
    /// runs, so the publish happens inside it.
    pub fn with_events(repository: R, events: E) -> Self {
        OrdersController(Arc::new(OrdersService {
            repository,
            ids: SystemIds,
            uow: DirectUnitOfWork,
            events,
        }))
    }
}

#[controller]
impl<R, E> Orders for OrdersController<R, E>
where
    R: OrderRepository + Send + Sync + 'static,
    E: EventPublisher + 'static,
{
    async fn place(&self, cmd: PlaceOrder) -> Result<OrderDto, OrdersError> {
        self.0.place(cmd).await
    }

    async fn get(&self, id: OrderId) -> Result<OrderDto, OrdersError> {
        self.0.get(id).await
    }

    async fn list(&self) -> Result<Vec<OrderDto>, OrdersError> {
        self.0.list().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use seiva_http::{Principal, PROBLEM_JSON};
    use serde_json::Value;
    use shop_orders_infrastructure::InMemoryOrderRepository;
    use tower::ServiceExt;

    async fn json_body(body: Body) -> Value {
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn demo_router() -> axum::Router {
        orders_router(OrdersController::new(InMemoryOrderRepository::new()))
    }

    #[tokio::test]
    async fn place_is_forbidden_without_permission() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/orders/place")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"customer":"Ada","lines":[{"product_id":"10","qty":2}]}"#,
            ))
            .unwrap();

        let response = demo_router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = json_body(response.into_body()).await;
        assert!(body["detail"].as_str().unwrap().contains("orders.manage"));
    }

    #[tokio::test]
    async fn place_with_no_lines_is_422_problem() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/orders/place")
            .header("content-type", "application/json")
            .extension(Principal::new("mgr", ["orders.manage"]))
            .body(Body::from(r#"{"customer":"Ada","lines":[]}"#))
            .unwrap();

        let response = demo_router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.headers()["content-type"], PROBLEM_JSON);
    }

    #[tokio::test]
    async fn place_then_get_and_list_round_trip() {
        let app = demo_router();

        let place = Request::builder()
            .method("POST")
            .uri("/v1/orders/place")
            .header("content-type", "application/json")
            .extension(Principal::new("mgr", ["orders.manage"]))
            .body(Body::from(
                r#"{"customer":"Ada","lines":[{"product_id":"10","qty":2}]}"#,
            ))
            .unwrap();
        let response = app.clone().oneshot(place).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let created = json_body(response.into_body()).await;
        let id = created["id"].as_str().unwrap();

        let get = Request::builder()
            .method("GET")
            .uri(format!("/v1/orders/{id}"))
            .extension(Principal::new("clerk", ["orders.read"]))
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(get).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let fetched = json_body(response.into_body()).await;
        assert_eq!(fetched["customer"], "Ada");
        assert_eq!(fetched["status"], "placed");

        let list = Request::builder()
            .method("GET")
            .uri("/v1/orders")
            .extension(Principal::new("clerk", ["orders.read"]))
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(list).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let listed = json_body(response.into_body()).await;
        assert_eq!(listed.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn get_unknown_order_is_404_problem() {
        let request = Request::builder()
            .method("GET")
            .uri("/v1/orders/404")
            .extension(Principal::new("clerk", ["orders.read"]))
            .body(Body::empty())
            .unwrap();

        let response = demo_router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()["content-type"], PROBLEM_JSON);
    }
}
