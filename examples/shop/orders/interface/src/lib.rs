//! Interface layer of the `orders` module: the HTTP transport.
//!
//! The `#[seiva::controller]` lives HERE; the controller [`OrdersController`] is
//! generic over the repository port `R`, the event publisher `E` and the
//! idempotency store `S`, so this layer depends only on `application` and never on
//! `infrastructure`. The composition root injects the concrete ports and merges
//! `orders_router(controller)`.
//!
//! # Dispatch through the command bus (ADR 0020 Fase 3)
//!
//! `place` does **not** call the service directly: it builds and dispatches the
//! canonical write chain `Validate → Transactional{ Idempotent → handler }` (the
//! inner `Idempotent` link present only when a store is injected and the request
//! carries an `Idempotency-Key`). The declarative chain replaces the module's former
//! hand-rolled transaction + idempotency boilerplate — see [`OrdersController`].
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

use axum::http::StatusCode;
use delonix_seiva::admin::{AdminPermissions, AdminResource};
use delonix_seiva::{controller, register_admin};
use seiva_cqrs::{CommandHandler, Mediator, Transactional, Validate};
use seiva_http::{Ctx, Idempotent, Problem, DEFAULT_IDEMPOTENCY_TTL};
use seiva_ports::{BoxError, EventPublisher, IdempotencyStore, PortResult};

use shop_orders_application::cqrs::PlaceOrderCommand;
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

use runtime::{DirectUnitOfWork, NoIdempotency, NoopEventPublisher, SystemIds};

/// In-process ports that let the interface compose a runnable `orders` service
/// without the DORM data layer.
mod runtime {
    use async_trait::async_trait;
    use seiva_ports::{
        EventPublisher, Id, IdGenerator, IdempotencyOutcome, IdempotencyStore, PortResult,
        UnitOfWork,
    };
    use std::future::Future;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

    /// The **absent** idempotency store: the default `S` for a controller built
    /// without durable idempotency ([`new`](super::OrdersController::new) /
    /// [`with_events`](super::OrdersController::with_events)).
    ///
    /// It exists only to give the controller a concrete store type when none is
    /// injected; its methods are **never called**, because `place` only builds the
    /// `Idempotent` behavior when the controller actually holds a store
    /// (`Option<S>` is `Some`). A controller carrying this type therefore dispatches
    /// `Validate → Transactional{ handler }` — the chain without the dedup link. The
    /// methods are implemented as a safe no-op (every request is `Fresh`) rather than
    /// a panic, so the type is sound even in the unreachable branch the type checker
    /// still has to accept.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct NoIdempotency;

    #[async_trait]
    impl IdempotencyStore for NoIdempotency {
        async fn begin(
            &self,
            _scoped_key: &str,
            _fingerprint: &str,
            _ttl: Duration,
        ) -> PortResult<IdempotencyOutcome> {
            Ok(IdempotencyOutcome::Fresh)
        }

        async fn finish(&self, _scoped_key: &str, _response: &[u8]) -> PortResult<()> {
            Ok(())
        }
    }
}

/// The concrete `orders` service this controller wraps, generic over the event
/// publisher port `E` so the composition root can swap the default no-op for a real
/// one (an outbox) without touching the module.
type Service<R, E> = OrdersService<R, SystemIds, DirectUnitOfWork, E>;

/// The transaction-free command handler the bus chain nests: it runs the use case
/// body ([`OrdersService::place_work`]) **without** opening a transaction, so the
/// enclosing `Transactional` owns the one transaction and the inner `Idempotent`
/// (when present) the dedup — the ADR 0020 write order.
///
/// It maps the service's [`OrdersError`] to the port error as a **status-bearing**
/// [`Problem`] (`422`/`404`), so the status survives the round trip back through the
/// bus and [`bus_err_to_orders`] can recover it rather than flattening it to a
/// generic `500`.
struct PlaceWorkHandler<R, E> {
    service: Arc<Service<R, E>>,
}

impl<R, E> PlaceWorkHandler<R, E> {
    fn new(service: Arc<Service<R, E>>) -> Self {
        Self { service }
    }
}

impl<R, E> CommandHandler<PlaceOrderCommand> for PlaceWorkHandler<R, E>
where
    R: OrderRepository + Send + Sync + 'static,
    E: EventPublisher + 'static,
{
    async fn handle(&self, cmd: PlaceOrderCommand) -> PortResult<OrderDto> {
        self.service
            .place_work(cmd.0)
            .await
            .map_err(|e| BoxError::from(Problem::from(e)))
    }
}

/// Recovers an [`OrdersError`] from a bus [`PortResult`] error, **preserving the
/// status** a behavior produced.
///
/// Every error that travels back through the bus is turned into a [`Problem`] by the
/// downcast-aware [`Problem::from`]: a status-bearing one (a `409` from `Idempotent`,
/// the `422`/`404` the handler boxed) is recovered verbatim, an opaque one becomes a
/// `500`. We then re-map the status onto the module's own error enum so the generated
/// controller renders exactly that status — in particular an idempotency `409`
/// becomes [`OrdersError::Conflict`] rather than being collapsed into a `422`.
fn bus_err_to_orders(err: BoxError) -> OrdersError {
    let problem = Problem::from(err);
    let detail = problem
        .detail
        .as_deref()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| problem.title.to_string());
    match problem.status {
        StatusCode::CONFLICT => OrdersError::Conflict(detail),
        StatusCode::NOT_FOUND => OrdersError::NotFound(detail),
        _ => OrdersError::Invalid(detail),
    }
}

/// Transport adapter: the `orders` service exposed over HTTP, generic over the
/// repository port `R`, the event publisher port `E` and the idempotency store `S`.
///
/// # Generics
///
/// - `E` defaults to `NoopEventPublisher`, so [`new`](Self::new) stays a one-arg
///   constructor for the standalone demo where `orders.placed` is swallowed. The
///   composition root that wants the event to flow (ADR 0021) injects a real
///   publisher through [`with_events`](Self::with_events) — e.g. an in-memory outbox.
/// - `S` defaults to `NoIdempotency`, the absent store. When no store is injected
///   the controller holds `idem: None` and dispatches the chain **without** the
///   dedup link. A composition root that wants durable, cross-replica idempotency
///   (ADR 0011) injects a store through [`with_idempotency`](Self::with_idempotency).
///
/// # How `place` dispatches
///
/// `place` routes the write through the ADR 0020 command bus instead of calling the
/// service directly: `Validate → Transactional{ Idempotent → handler }` (or, with no
/// `Idempotency-Key`/no store, `Validate → Transactional{ handler }`). The nesting
/// order is a correctness invariant: `Validate` sits **outside** `Transactional` so a
/// malformed request opens no transaction; `Idempotent` sits **inside** it so the key
/// reservation, the save and the outbox publish commit as one atom (ADR 0011). This
/// replaces the module's previous hand-rolled transaction with a declarative chain.
/// Authorization is enforced *before* this method by the generated route's permission
/// guard (the facade's `#[requires("orders.manage")]`), so `Authorize` is not part of
/// the in-method chain.
pub struct OrdersController<R, E = NoopEventPublisher, S = NoIdempotency> {
    service: Arc<Service<R, E>>,
    /// The durable idempotency store, or `None` for the chain without the dedup link.
    idem: Option<S>,
}

impl<R, E, S: Clone> Clone for OrdersController<R, E, S> {
    fn clone(&self) -> Self {
        Self {
            service: Arc::clone(&self.service),
            idem: self.idem.clone(),
        }
    }
}

impl<R: OrderRepository> OrdersController<R, NoopEventPublisher, NoIdempotency> {
    /// Builds the controller over `repository` and the in-process ports, with the
    /// `orders.placed` event swallowed by a `NoopEventPublisher` and **no** durable
    /// idempotency (the dedup link is omitted). The runnable default for the
    /// standalone module demo.
    pub fn new(repository: R) -> Self {
        Self::with_events(repository, NoopEventPublisher)
    }
}

impl<R: OrderRepository, E: EventPublisher> OrdersController<R, E, NoIdempotency> {
    /// Builds the controller over `repository` and an injected event publisher
    /// `events`, so a published `orders.placed` lands wherever `events` writes it
    /// (an outbox the composition root then dispatches). Still **without** durable
    /// idempotency; the write runs through `Validate → Transactional{ handler }`.
    pub fn with_events(repository: R, events: E) -> Self {
        OrdersController {
            service: Arc::new(OrdersService {
                repository,
                ids: SystemIds,
                uow: DirectUnitOfWork,
                events,
            }),
            idem: None,
        }
    }
}

impl<R: OrderRepository, E: EventPublisher, S: IdempotencyStore> OrdersController<R, E, S> {
    /// Builds the controller over `repository`, an injected event publisher `events`
    /// and a durable idempotency store `store`, so a `place` carrying an
    /// `Idempotency-Key` dedupes through the bus's `Idempotent` behavior — a retried
    /// write produces exactly one order and one `orders.placed` event (ADR 0011). A
    /// request without the header still runs, just without the dedup link.
    pub fn with_idempotency(repository: R, events: E, store: S) -> Self {
        OrdersController {
            service: Arc::new(OrdersService {
                repository,
                ids: SystemIds,
                uow: DirectUnitOfWork,
                events,
            }),
            idem: Some(store),
        }
    }
}

#[controller]
impl<R, E, S> Orders for OrdersController<R, E, S>
where
    R: OrderRepository + Send + Sync + 'static,
    E: EventPublisher + 'static,
    S: IdempotencyStore + Clone + 'static,
{
    async fn place(&self, ctx: &Ctx, cmd: PlaceOrder) -> Result<OrderDto, OrdersError> {
        // The bytes `Idempotent` fingerprints: the canonical JSON of the command, so
        // a retry with the same key but different content is a mismatch (409).
        let body = serde_json::to_value(&cmd).map_err(|e| OrdersError::Invalid(e.to_string()))?;

        // Build the dedup link only when BOTH a store is injected AND the request
        // carries an `Idempotency-Key` (opt-in per request, ADR 0011). `from_ctx`
        // returns `None` without a key, so a plain request skips the link entirely.
        let idem = self.idem.as_ref().and_then(|store| {
            Idempotent::from_ctx(
                PlaceWorkHandler::new(Arc::clone(&self.service)),
                store.clone(),
                ctx,
                &body,
                DEFAULT_IDEMPOTENCY_TTL,
            )
        });

        // Dispatch through the ADR 0020 chain. With the dedup link:
        // `Validate → Transactional{ Idempotent → handler }`; without it:
        // `Validate → Transactional{ handler }`. `Transactional` opens its own
        // (in-process) transaction, so `place_work` must not.
        let result = match idem {
            Some(idem) => {
                let chain = Validate::new(Transactional::new(idem, DirectUnitOfWork));
                Mediator::new(chain).send(PlaceOrderCommand(cmd)).await
            }
            None => {
                let handler = PlaceWorkHandler::new(Arc::clone(&self.service));
                let chain = Validate::new(Transactional::new(handler, DirectUnitOfWork));
                Mediator::new(chain).send(PlaceOrderCommand(cmd)).await
            }
        };
        result.map_err(bus_err_to_orders)
    }

    async fn get(&self, id: OrderId) -> Result<OrderDto, OrdersError> {
        self.service.get(id).await
    }

    async fn list(&self) -> Result<Vec<OrderDto>, OrdersError> {
        self.service.list().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use seiva_http::{IdempotencyKey, Principal, PROBLEM_JSON};
    use seiva_testing::InMemoryIdempotencyStore;
    use serde_json::Value;
    use shop_orders_domain::Order;
    use shop_orders_facade::LineDto;
    use shop_orders_infrastructure::InMemoryOrderRepository;
    use std::sync::Mutex;
    use tower::ServiceExt;

    async fn json_body(body: Body) -> Value {
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn demo_router() -> axum::Router {
        orders_router(OrdersController::new(InMemoryOrderRepository::new()))
    }

    /// A repository that shares its store behind an `Arc`, so a test keeps a probe
    /// handle to count how many orders the handler actually saved — the way to prove
    /// a behavior short-circuited (0 saves) or deduplicated (1 save across two calls).
    #[derive(Clone, Default)]
    struct CountingRepo {
        orders: Arc<Mutex<Vec<Order>>>,
    }

    impl OrderRepository for CountingRepo {
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

    fn line(product_id: u128, qty: u64) -> LineDto {
        LineDto {
            product_id: product_id.into(),
            qty,
        }
    }

    /// A `Ctx` as the transport would build it for `POST /v1/orders/place`: an
    /// `orders.manage` principal and an optional `Idempotency-Key`.
    fn ctx_for(key: Option<&str>) -> Ctx {
        let key = key.map(|raw| IdempotencyKey::parse(raw).expect("valid test key"));
        Ctx::new(Principal::new("mgr", ["orders.manage"])).with_request(
            "POST",
            "/v1/orders/place",
            key,
        )
    }

    #[tokio::test]
    async fn the_bus_validate_rejects_an_invalid_command_and_the_handler_never_runs() {
        let repo = CountingRepo::default();
        let probe = repo.clone();
        let controller = OrdersController::with_idempotency(
            repo,
            NoopEventPublisher,
            InMemoryIdempotencyStore::new(),
        );

        // An order with no lines fails the command's `Validated` check, which sits
        // OUTSIDE `Transactional` — so it is rejected before the handler runs.
        let result = controller
            .place(
                &ctx_for(Some("k-invalid")),
                PlaceOrder {
                    customer: "Ada".into(),
                    lines: vec![],
                },
            )
            .await;

        assert!(result.is_err(), "an invalid command must be rejected");
        assert!(
            probe.orders.lock().unwrap().is_empty(),
            "the handler never ran: no order was saved"
        );
    }

    #[tokio::test]
    async fn a_retry_with_the_same_idempotency_key_places_exactly_one_order() {
        let repo = CountingRepo::default();
        let probe = repo.clone();
        let controller = OrdersController::with_idempotency(
            repo,
            NoopEventPublisher,
            InMemoryIdempotencyStore::new(),
        );
        let ctx = ctx_for(Some("retry-key"));
        let cmd = PlaceOrder {
            customer: "Ada".into(),
            lines: vec![line(10, 2)],
        };

        let first = controller
            .place(&ctx, cmd.clone())
            .await
            .expect("first place");
        let second = controller.place(&ctx, cmd.clone()).await.expect("retry");

        // The handler ran once; the retry replayed the stored response through the bus.
        assert_eq!(first, second, "the retry replays the first response");
        assert_eq!(
            probe.orders.lock().unwrap().len(),
            1,
            "the bus dedup produced exactly one order"
        );
    }

    #[tokio::test]
    async fn the_same_key_with_different_content_is_a_409_conflict() {
        let repo = CountingRepo::default();
        let probe = repo.clone();
        let controller = OrdersController::with_idempotency(
            repo,
            NoopEventPublisher,
            InMemoryIdempotencyStore::new(),
        );
        let ctx = ctx_for(Some("same-key"));

        controller
            .place(
                &ctx,
                PlaceOrder {
                    customer: "Ada".into(),
                    lines: vec![line(10, 2)],
                },
            )
            .await
            .expect("first place");

        // Same key, different (still valid) content → fingerprint mismatch → 409.
        let conflict = controller
            .place(
                &ctx,
                PlaceOrder {
                    customer: "Ada".into(),
                    lines: vec![line(10, 99)],
                },
            )
            .await;

        assert!(
            matches!(conflict, Err(OrdersError::Conflict(_))),
            "a key reused with different content is a 409 conflict, got {conflict:?}"
        );
        assert_eq!(
            probe.orders.lock().unwrap().len(),
            1,
            "the mismatched retry never ran the handler"
        );
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
