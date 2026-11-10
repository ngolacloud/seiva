//! Interface layer of the `notifications` module: the HTTP transport.
//!
//! The `#[seiva::controller]` lives HERE; the controller
//! [`NotificationsController<R>`] is generic over the repository port `R`, so this
//! layer depends only on `application` and never on `infrastructure`. The
//! composition root injects the concrete repository and merges
//! `notifications_router(controller)`.
//!
//! # Generated routes
//!
//! | facade method | route                           | success | required permission   |
//! |---------------|---------------------------------|---------|-----------------------|
//! | `send`        | `POST /v1/notifications/send`   | `200`   | `notifications.send`  |
//! | `list`        | `GET  /v1/notifications`        | `200`   | `notifications.read`  |

#![forbid(unsafe_code)]

use std::sync::Arc;

use delonix_seiva::admin::{AdminPermissions, AdminResource};
use delonix_seiva::{controller, register_admin};

use shop_notifications_application::{NotificationRepository, NotificationsService};
use shop_notifications_facade::{
    NotificationDto, Notifications, NotificationsError, NotificationsFacade, SendNotification,
};

register_admin!(AdminResource {
    namespace: "notifications",
    resource: "notifications",
    label: "Notifications",
    facade: "Notifications",
    version: "v1",
    list_fields: &["id", "to", "subject"],
    search_fields: &["to", "subject"],
    filters: &[],
    ordering: &["id"],
    page_size: 25,
    permissions: AdminPermissions {
        view: "notifications.read",
        create: "notifications.send",
        change: "",
        delete: "",
    },
});

use runtime::{DirectUnitOfWork, NoopEventPublisher, SystemIds};

/// In-process ports that let the interface compose a runnable `notifications`
/// service without the DORM data layer.
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

/// The concrete `Notifications` service this controller wraps.
type Service<R> = NotificationsService<R, SystemIds, DirectUnitOfWork, NoopEventPublisher>;

/// Transport adapter: a concrete `Notifications` service exposed over HTTP,
/// generic over the repository port `R`.
pub struct NotificationsController<R>(Arc<Service<R>>);

impl<R> Clone for NotificationsController<R> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<R: NotificationRepository> NotificationsController<R> {
    /// Builds the controller over `repository` and the in-process ports.
    pub fn new(repository: R) -> Self {
        NotificationsController(Arc::new(NotificationsService {
            repository,
            ids: SystemIds,
            uow: DirectUnitOfWork,
            events: NoopEventPublisher,
        }))
    }
}

#[controller]
impl<R> Notifications for NotificationsController<R>
where
    R: NotificationRepository + Send + Sync + 'static,
{
    async fn send(&self, cmd: SendNotification) -> Result<NotificationDto, NotificationsError> {
        self.0.send(cmd).await
    }

    async fn list(&self) -> Result<Vec<NotificationDto>, NotificationsError> {
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
    use shop_notifications_infrastructure::InMemoryNotificationRepository;
    use tower::ServiceExt;

    async fn json_body(body: Body) -> Value {
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn demo_router() -> axum::Router {
        notifications_router(NotificationsController::new(
            InMemoryNotificationRepository::new(),
        ))
    }

    #[tokio::test]
    async fn send_is_forbidden_without_permission() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/notifications/send")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"to":"ada@example.com","subject":"Hi","body":"Hello"}"#,
            ))
            .unwrap();

        let response = demo_router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = json_body(response.into_body()).await;
        assert!(body["detail"]
            .as_str()
            .unwrap()
            .contains("notifications.send"));
    }

    #[tokio::test]
    async fn send_empty_recipient_is_422_problem() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/notifications/send")
            .header("content-type", "application/json")
            .extension(Principal::new("sender", ["notifications.send"]))
            .body(Body::from(r#"{"to":"  ","subject":"Hi","body":"Hello"}"#))
            .unwrap();

        let response = demo_router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.headers()["content-type"], PROBLEM_JSON);
    }

    #[tokio::test]
    async fn send_then_list_round_trip() {
        let app = demo_router();

        let send = Request::builder()
            .method("POST")
            .uri("/v1/notifications/send")
            .header("content-type", "application/json")
            .extension(Principal::new("sender", ["notifications.send"]))
            .body(Body::from(
                r#"{"to":"ada@example.com","subject":"Order placed","body":"Thanks!"}"#,
            ))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(send).await.unwrap().status(),
            StatusCode::OK
        );

        let list = Request::builder()
            .method("GET")
            .uri("/v1/notifications")
            .extension(Principal::new("clerk", ["notifications.read"]))
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(list).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let listed = json_body(response.into_body()).await;
        assert_eq!(listed.as_array().unwrap().len(), 1);
    }
}
