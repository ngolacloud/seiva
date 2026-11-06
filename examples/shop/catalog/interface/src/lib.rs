//! Interface layer of the `catalog` module: the HTTP transport.
//!
//! The `#[seiva::controller]` lives HERE, not in `application`, so the
//! application layer stays transport-free. The controller [`CatalogController<R>`]
//! is GENERIC over the repository port `R`, so this layer depends only on
//! `application` (for the service and the `ProductRepository` port) and never on
//! the `infrastructure` crate. The composition root picks the concrete repository
//! and merges `catalog_router(controller)` into its application.
//!
//! # Generated routes
//!
//! The controller keys the HTTP method and path off the facade **method name**:
//!
//! | facade method | route                              | success | required permission |
//! |---------------|------------------------------------|---------|---------------------|
//! | `add_product` | `POST /v1/catalogs/add_product`    | `200`   | `catalog.manage`    |
//! | `list`        | `GET  /v1/catalogs`                | `200`   | `catalog.read`      |
//! | `get`         | `GET  /v1/catalogs/{id}`           | `200`   | `catalog.read`      |

#![forbid(unsafe_code)]

use std::sync::Arc;

use delonix_seiva::admin::{AdminPermissions, AdminResource};
use delonix_seiva::{controller, register_admin};

use shop_catalog_application::{CatalogService, ProductRepository};
use shop_catalog_facade::{
    Catalog, CatalogError, CatalogFacade, CreateProduct, ProductDto, ProductId,
};

// Make `catalog` visible to the optional admin console (ADR 0009). This couples
// the module only to the framework contract (`delonix-seiva`), NOT to the admin
// UI crate: with the console unmounted this is inert link-time data. The resource
// id matches the module's route segment (`/v1/catalogs/...`) and each permission
// matches the facade's `#[requires(..)]`.
register_admin!(AdminResource {
    namespace: "catalog",
    resource: "catalogs",
    label: "Catalog",
    facade: "Catalog",
    version: "v1",
    list_fields: &["id", "name", "price_cents"],
    search_fields: &["name"],
    filters: &[],
    ordering: &["name", "price_cents"],
    page_size: 25,
    permissions: AdminPermissions {
        view: "catalog.read",
        create: "catalog.manage",
        change: "",
        delete: "",
    },
});

use runtime::{DirectUnitOfWork, NoopEventPublisher, SystemIds};

/// In-process ports that let the interface compose a runnable `catalog` service
/// without the DORM data layer.
mod runtime {
    use async_trait::async_trait;
    use seiva_ports::{EventPublisher, Id, IdGenerator, PortResult, UnitOfWork};
    use std::future::Future;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A time-seeded [`IdGenerator`]: the 16 id bytes are the current time in
    /// nanoseconds, big-endian.
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

    /// A pass-through [`UnitOfWork`]: runs the use-case body directly.
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

    /// A no-op [`EventPublisher`]: discards the emitted event.
    pub struct NoopEventPublisher;

    #[async_trait]
    impl EventPublisher for NoopEventPublisher {
        async fn publish(&self, _topic: &str, _payload: &[u8]) -> PortResult<()> {
            Ok(())
        }
    }
}

/// The concrete `Catalog` service this controller wraps: the application service
/// over a caller-supplied repository `R` and the in-process `runtime` ports.
type Service<R> = CatalogService<R, SystemIds, DirectUnitOfWork, NoopEventPublisher>;

/// Transport adapter: a concrete `Catalog` service exposed over HTTP, generic over
/// the repository port `R`. Holds the service behind an [`Arc`] so it is
/// `Clone + Send + Sync + 'static` — the bound the generated `catalog_router`
/// places on its axum state.
pub struct CatalogController<R>(Arc<Service<R>>);

impl<R> Clone for CatalogController<R> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<R: ProductRepository> CatalogController<R> {
    /// Builds the controller over `repository` and the in-process ports.
    pub fn new(repository: R) -> Self {
        CatalogController(Arc::new(CatalogService {
            repository,
            ids: SystemIds,
            uow: DirectUnitOfWork,
            events: NoopEventPublisher,
        }))
    }
}

#[controller]
impl<R> Catalog for CatalogController<R>
where
    R: ProductRepository + Send + Sync + 'static,
{
    async fn add_product(&self, cmd: CreateProduct) -> Result<ProductDto, CatalogError> {
        self.0.add_product(cmd).await
    }

    async fn list(&self) -> Result<Vec<ProductDto>, CatalogError> {
        self.0.list().await
    }

    async fn get(&self, id: ProductId) -> Result<ProductDto, CatalogError> {
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
    use shop_catalog_infrastructure::InMemoryProductRepository;
    use tower::ServiceExt;

    async fn json_body(body: Body) -> Value {
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// The composition the real `apps/` root performs: the generated
    /// `catalog_router` over a controller wired to the in-memory repository.
    fn demo_router() -> axum::Router {
        catalog_router(CatalogController::new(InMemoryProductRepository::new()))
    }

    #[tokio::test]
    async fn add_route_is_forbidden_without_permission() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/catalogs/add_product")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"Teapot","price_cents":1299}"#))
            .unwrap();

        let response = demo_router().oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.headers()["content-type"], PROBLEM_JSON);
        let body = json_body(response.into_body()).await;
        assert_eq!(body["status"], 403);
        assert!(body["detail"].as_str().unwrap().contains("catalog.manage"));
    }

    #[tokio::test]
    async fn add_route_invalid_body_is_422_problem() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/catalogs/add_product")
            .header("content-type", "application/json")
            .extension(Principal::new("mgr", ["catalog.manage"]))
            .body(Body::from(r#"{"name":"  ","price_cents":1299}"#))
            .unwrap();

        let response = demo_router().oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.headers()["content-type"], PROBLEM_JSON);
    }

    #[tokio::test]
    async fn add_then_get_and_list_round_trip() {
        let app = demo_router();

        let add = Request::builder()
            .method("POST")
            .uri("/v1/catalogs/add_product")
            .header("content-type", "application/json")
            .extension(Principal::new("mgr", ["catalog.manage"]))
            .body(Body::from(r#"{"name":"Teapot","price_cents":1299}"#))
            .unwrap();
        let response = app.clone().oneshot(add).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let created = json_body(response.into_body()).await;
        let id = created["id"].as_u64().unwrap();

        let get = Request::builder()
            .method("GET")
            .uri(format!("/v1/catalogs/{id}"))
            .extension(Principal::new("clerk", ["catalog.read"]))
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(get).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let fetched = json_body(response.into_body()).await;
        assert_eq!(fetched["name"], "Teapot");

        let list = Request::builder()
            .method("GET")
            .uri("/v1/catalogs")
            .extension(Principal::new("clerk", ["catalog.read"]))
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(list).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let listed = json_body(response.into_body()).await;
        assert_eq!(listed.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn get_unknown_product_is_404_problem() {
        let request = Request::builder()
            .method("GET")
            .uri("/v1/catalogs/404")
            .extension(Principal::new("clerk", ["catalog.read"]))
            .body(Body::empty())
            .unwrap();

        let response = demo_router().oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()["content-type"], PROBLEM_JSON);
    }
}
