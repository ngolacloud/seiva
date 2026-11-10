//! Interface layer of the `payments` module: the HTTP transport.
//!
//! The `#[seiva::controller]` lives HERE; the controller [`PaymentsController<R>`]
//! is generic over the repository port `R`, so this layer depends only on
//! `application` and never on `infrastructure`. The composition root injects the
//! concrete repository and merges `payments_router(controller)`.
//!
//! # Generated routes
//!
//! | facade method | route                      | success | required permission |
//! |---------------|----------------------------|---------|---------------------|
//! | `charge`      | `POST /v1/payments/charge` | `200`   | `payments.charge`   |
//! | `get`         | `GET  /v1/payments/{id}`   | `200`   | `payments.read`     |

#![forbid(unsafe_code)]

use std::sync::Arc;

use delonix_seiva::admin::{AdminPermissions, AdminResource};
use delonix_seiva::{controller, register_admin};

use shop_payments_application::{PaymentRepository, PaymentsService};
use shop_payments_facade::{
    Charge, PaymentDto, PaymentId, Payments, PaymentsError, PaymentsFacade,
};

register_admin!(AdminResource {
    namespace: "payments",
    resource: "payments",
    label: "Payments",
    facade: "Payments",
    version: "v1",
    list_fields: &["id", "order_id", "amount_cents", "status"],
    search_fields: &[],
    filters: &["status"],
    ordering: &["id"],
    page_size: 25,
    permissions: AdminPermissions {
        view: "payments.read",
        create: "payments.charge",
        change: "",
        delete: "",
    },
});

use runtime::{DirectUnitOfWork, NoopEventPublisher, SystemIds};

/// In-process ports that let the interface compose a runnable `payments` service
/// without the DORM data layer.
mod runtime {
    use async_trait::async_trait;
    use seiva_ports::{EventPublisher, Id, IdGenerator, PortResult, Scope, UnitOfWork};
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
            _scope: Scope,
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

/// The concrete `Payments` service this controller wraps.
type Service<R> = PaymentsService<R, SystemIds, DirectUnitOfWork, NoopEventPublisher>;

/// Transport adapter: a concrete `Payments` service exposed over HTTP, generic
/// over the repository port `R`.
pub struct PaymentsController<R>(Arc<Service<R>>);

impl<R> Clone for PaymentsController<R> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<R: PaymentRepository> PaymentsController<R> {
    /// Builds the controller over `repository` and the in-process ports.
    pub fn new(repository: R) -> Self {
        PaymentsController(Arc::new(PaymentsService {
            repository,
            ids: SystemIds,
            uow: DirectUnitOfWork,
            events: NoopEventPublisher,
        }))
    }
}

#[controller]
impl<R> Payments for PaymentsController<R>
where
    R: PaymentRepository + Send + Sync + 'static,
{
    async fn charge(&self, cmd: Charge) -> Result<PaymentDto, PaymentsError> {
        self.0.charge(cmd).await
    }

    async fn get(&self, id: PaymentId) -> Result<PaymentDto, PaymentsError> {
        self.0.get(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use seiva_http::{Principal, PROBLEM_JSON};
    use serde_json::Value;
    use shop_payments_infrastructure::InMemoryPaymentRepository;
    use tower::ServiceExt;

    async fn json_body(body: Body) -> Value {
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn demo_router() -> axum::Router {
        payments_router(PaymentsController::new(InMemoryPaymentRepository::new()))
    }

    #[tokio::test]
    async fn charge_is_forbidden_without_permission() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/payments/charge")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"order_id":"7","amount_cents":2500}"#))
            .unwrap();

        let response = demo_router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = json_body(response.into_body()).await;
        assert!(body["detail"].as_str().unwrap().contains("payments.charge"));
    }

    #[tokio::test]
    async fn charge_then_get_round_trip() {
        let app = demo_router();

        let charge = Request::builder()
            .method("POST")
            .uri("/v1/payments/charge")
            .header("content-type", "application/json")
            .extension(Principal::new("biller", ["payments.charge"]))
            .body(Body::from(r#"{"order_id":"7","amount_cents":2500}"#))
            .unwrap();
        let response = app.clone().oneshot(charge).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let created = json_body(response.into_body()).await;
        assert_eq!(created["status"], "captured");
        let id = created["id"].as_str().unwrap();

        let get = Request::builder()
            .method("GET")
            .uri(format!("/v1/payments/{id}"))
            .extension(Principal::new("clerk", ["payments.read"]))
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(get).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let fetched = json_body(response.into_body()).await;
        assert_eq!(fetched["amount_cents"], 2500);
    }

    #[tokio::test]
    async fn get_unknown_payment_is_404_problem() {
        let request = Request::builder()
            .method("GET")
            .uri("/v1/payments/404")
            .extension(Principal::new("clerk", ["payments.read"]))
            .body(Body::empty())
            .unwrap();

        let response = demo_router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()["content-type"], PROBLEM_JSON);
    }
}
