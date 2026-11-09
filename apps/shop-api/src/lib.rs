//! Composition root for the `shop` tutorial — the library half.
//!
//! It composes the five INDEPENDENT shop modules — catalog, inventory, orders,
//! payments, notifications — into one running HTTP service, and it wires the
//! **live choreography** of ADR 0021 Fase 3: an `orders.placed` event, published
//! by the `orders` module into an in-process outbox, is dispatched to an
//! `inventory` subscriber that reserves the ordered stock — with **no database and
//! no broker**, exactly the offline twin of the broker path.
//!
//! The binary ([`main`](../main.rs)) is a thin shell over [`build_app`]; keeping
//! the wiring in a library target is what lets `tests/choreography_e2e.rs` drive
//! the very same composition (HTTP in, outbox drained, stock asserted).
//!
//! # How the choreography is wired
//!
//! - **One shared stock state.** The `inventory` repository is built once behind an
//!   [`Arc`] and injected BOTH into the `inventory` HTTP controller and into the
//!   event subscriber (through the `Arc<R>: StockRepository` delegation). So the
//!   stock a `POST /v1/inventories/receive` seeds is the stock the subscriber
//!   reserves and the stock a later query reads — one state, two entry points.
//! - **A real publisher in `orders`.** The `orders` controller is built with
//!   [`InMemoryOutboxPublisher`] over a shared [`InMemoryOutbox`] (replacing the
//!   module's default no-op), so `place` writes a real `orders.placed` CloudEvent
//!   into the outbox inside its use-case transaction.
//! - **A subscriber at the ROOT, not in a module.** [`InventorySubscriber`] lives
//!   here in the composition root — never inside the `orders` or `inventory`
//!   crates — so the two modules stay decoupled: the root alone routes the event
//!   type to the handler, which is exactly what makes `seiva module split` a
//!   composition change rather than a rewrite.
//! - **A drainable outbox.** [`build_app`] hands back the [`InMemoryOutbox`], the
//!   [`SubscriberRegistry`] and the [`InMemoryIdempotencyStore`] so a caller drives
//!   [`drain_in_process`](seiva_events::drain_in_process) to dispatch pending
//!   events (the in-process twin of the broker drain), deduped through the inbox.
//!
//! A valid demo bearer is `shop-token` → a principal holding every shop
//! permission. Bind address from `SHOP_API_ADDR` (default `127.0.0.1:8090`).

#![forbid(unsafe_code)]

use std::sync::Arc;

use async_trait::async_trait;
use seiva_events::{EventSubscriber, InMemoryOutbox, InMemoryOutboxPublisher, SubscriberRegistry};
use seiva_http::{
    router_with_modules, Authenticator, Health, HttpSecurity, Principal, TokenAuthenticator,
};
use seiva_messaging::CloudEvent;
use seiva_ports::{BoxError, PortResult};
use seiva_testing::{
    InMemoryIdempotencyStore, InMemoryUnitOfWork, RecordingEventPublisher, SequenceIdGenerator,
};

use shop_catalog_interface::{catalog_router, CatalogController};
use shop_inventory_application::InventoryService;
use shop_inventory_facade::{Inventory, Reserve};
use shop_inventory_infrastructure::InMemoryStockRepository;
use shop_inventory_interface::{inventory_router, InventoryController};
use shop_notifications_interface::{notifications_router, NotificationsController};
use shop_orders_interface::{orders_router, OrdersController};
use shop_payments_interface::{payments_router, PaymentsController};

/// The `inventory` service the subscriber drives: the real `inventory` use case
/// over the SHARED stock repository, with offline doubles for the cross-cutting
/// ports (a sequence id generator, a commit-on-`Ok` unit of work, and a recording
/// publisher that swallows the resulting `inventory.reserved`). It reserves stock
/// through the same domain rules a `POST /v1/inventories/reserve` would.
type SharedInventory = InventoryService<
    Arc<InMemoryStockRepository>,
    SequenceIdGenerator,
    InMemoryUnitOfWork,
    RecordingEventPublisher,
>;

/// The composition-root subscriber that reacts to `orders.placed` by reserving the
/// ordered stock in the `inventory` module (ADR 0021 Fase 3).
///
/// It is deliberately defined HERE, in the binary's composition root, and NOT in
/// the `orders` or `inventory` crates: the two modules name no each other, and only
/// the root knows that an order placed should draw down inventory. The handler
/// deserialises the order lines carried in the event `data` and reserves each one
/// through the **real** [`Inventory`] use case over the shared stock, so the effect
/// is identical to an operator calling `reserve` over HTTP.
///
/// On a reservation it cannot satisfy (insufficient stock), [`handle`](Self::handle)
/// returns an error; the driver then does NOT advance the outbox cursor, so the
/// event is re-read on the next pass — at-least-once delivery, the same contract as
/// the broker path.
pub struct InventorySubscriber {
    inventory: SharedInventory,
}

impl InventorySubscriber {
    /// Builds the subscriber over the SHARED stock repository `repository` — the
    /// same `Arc` the `inventory` HTTP controller holds — so a reservation here is
    /// visible to every other reader of that stock.
    #[must_use]
    pub fn new(repository: Arc<InMemoryStockRepository>) -> Self {
        Self {
            inventory: InventoryService {
                repository,
                ids: SequenceIdGenerator::new(),
                uow: InMemoryUnitOfWork::new(),
                events: RecordingEventPublisher::new(),
            },
        }
    }
}

#[async_trait]
impl EventSubscriber for InventorySubscriber {
    fn subscriber_id(&self) -> &str {
        "inventory"
    }

    async fn handle(&self, event: &CloudEvent) -> PortResult<()> {
        // The `orders.placed` body carries the order lines the `orders` module
        // serialised: `{ id, customer, lines: [{ product_id: "<u128>", qty }] }`.
        let lines = event
            .data
            .get("lines")
            .and_then(|l| l.as_array())
            .ok_or_else(|| BoxError::from("orders.placed event has no `lines` array"))?;

        for line in lines {
            let product_id: u128 = line
                .get("product_id")
                .and_then(|p| p.as_str())
                .ok_or_else(|| BoxError::from("order line is missing a string `product_id`"))?
                .parse()
                .map_err(|err| {
                    BoxError::from(format!("order line `product_id` is not a u128: {err}"))
                })?;
            let qty = line
                .get("qty")
                .and_then(|q| q.as_u64())
                .ok_or_else(|| BoxError::from("order line is missing a numeric `qty`"))?;

            // Reserve through the real `inventory` use case over the shared stock.
            // A failure (e.g. insufficient stock) propagates so the driver leaves
            // the event pending for a later redelivery.
            self.inventory
                .reserve(Reserve {
                    product_id: product_id.into(),
                    qty,
                })
                .await
                .map_err(|err| BoxError::from(err.to_string()))?;
        }
        Ok(())
    }
}

/// Everything the composition root produces: the HTTP router plus the handles that
/// make the choreography drivable and observable offline.
///
/// A caller (`main`, or the e2e test) serves [`router`](Self::router) and, to make
/// the choreography progress, drains [`outbox`](Self::outbox) into
/// [`registry`](Self::registry)'s subscribers through
/// [`inbox`](Self::inbox) with
/// [`drain_in_process`](seiva_events::drain_in_process). The shared
/// [`inventory_repo`](Self::inventory_repo) is exposed so a test can read the stock
/// the subscriber mutated.
pub struct BuiltApp {
    /// The fully composed HTTP service (all five modules + the admin console),
    /// wrapped in the framework layers behind the demo token authenticator.
    pub router: axum::Router,
    /// The in-process outbox the `orders` module publishes `orders.placed` into;
    /// drive [`drain_in_process`](seiva_events::drain_in_process) over it.
    pub outbox: Arc<InMemoryOutbox>,
    /// The routing table mapping `orders.placed` to [`InventorySubscriber`].
    pub registry: SubscriberRegistry,
    /// The inbox that deduplicates each `(subscriber, event)` delivery, so a
    /// redelivery reserves stock exactly once.
    pub inbox: Arc<InMemoryIdempotencyStore>,
    /// The SHARED stock repository the inventory controller and the subscriber both
    /// hold — read it to observe the effect of a dispatched reservation.
    pub inventory_repo: Arc<InMemoryStockRepository>,
    /// The health handle, already marked ready; `main` passes it to the server.
    pub health: Health,
}

/// Composes the shop service and the live `orders.placed → inventory.reserved`
/// choreography, returning the router and the handles that drive and observe it.
///
/// See the [module docs](self) for how the stock is shared and the publisher
/// injected. The returned [`BuiltApp`] is ready to serve; the choreography only
/// progresses when a caller drains [`BuiltApp::outbox`].
#[must_use]
pub fn build_app() -> BuiltApp {
    // One shared stock state; one shared outbox; one shared inbox.
    let inventory_repo = Arc::new(InMemoryStockRepository::new());
    let outbox = Arc::new(InMemoryOutbox::new());
    let inbox = Arc::new(InMemoryIdempotencyStore::new());

    // The `inventory` HTTP controller over the SHARED repo (the `Arc<R>:
    // StockRepository` delegation makes the shared `Arc` a valid repository).
    let inventory_controller = InventoryController::new(Arc::clone(&inventory_repo));

    // The `orders` HTTP controller publishes a real `orders.placed` into the SHARED
    // outbox (replacing the module's default no-op publisher) AND dispatches `place`
    // through the ADR 0020 command bus with a durable idempotency store injected: a
    // retried `POST /v1/orders/place` carrying the same `Idempotency-Key` dedupes
    // through the bus's `Idempotent` behavior, so it produces exactly one order and
    // one `orders.placed` event. The store is dedicated to HTTP write idempotency,
    // distinct from the event-drain `inbox` above.
    let orders_controller = OrdersController::with_idempotency(
        shop_orders_infrastructure::InMemoryOrderRepository::new(),
        InMemoryOutboxPublisher::new(Arc::clone(&outbox), "/shop/orders"),
        InMemoryIdempotencyStore::new(),
    );

    let modules = catalog_router(CatalogController::new(
        shop_catalog_infrastructure::InMemoryProductRepository::new(),
    ))
    .merge(inventory_router(inventory_controller))
    .merge(orders_router(orders_controller))
    .merge(payments_router(PaymentsController::new(
        shop_payments_infrastructure::InMemoryPaymentRepository::new(),
    )))
    .merge(notifications_router(NotificationsController::new(
        shop_notifications_infrastructure::InMemoryNotificationRepository::new(),
    )));

    // Mount the console INTO modules so it is wrapped by the framework layers.
    let modules = seiva_console_admin::mount(modules);

    // The subscriber reacts to `orders.placed` over the SAME shared stock.
    let mut registry = SubscriberRegistry::new();
    registry.register(
        "orders.placed",
        Arc::new(InventorySubscriber::new(Arc::clone(&inventory_repo))),
    );

    let health = Health::starting();
    health.set_ready();

    // One demo token holding every shop permission. Permissions are matched
    // verbatim by the guard (no wildcards), so each `<module>.<action>` the facades
    // require is listed explicitly.
    let auth: Arc<dyn Authenticator> = Arc::new(TokenAuthenticator::new().with_token(
        "shop-token",
        Principal::new(
            "shopkeeper",
            [
                "catalog.read",
                "catalog.manage",
                "inventory.read",
                "inventory.manage",
                "orders.read",
                "orders.manage",
                "payments.read",
                "payments.charge",
                "notifications.read",
                "notifications.send",
            ],
        ),
    ));

    // CORS for a separately-deployed frontend: list the storefront's origin(s) in
    // SHOP_API_CORS_ORIGINS (comma-separated). Empty = same-origin only.
    let security = match std::env::var("SHOP_API_CORS_ORIGINS") {
        Ok(origins) if !origins.trim().is_empty() => {
            HttpSecurity::with_cors_origins(origins.split(',').map(|o| o.trim().to_owned()))
        }
        _ => HttpSecurity::default(),
    };

    let router = router_with_modules(health.clone(), &security, Some(auth), modules);

    BuiltApp {
        router,
        outbox,
        registry,
        inbox,
        inventory_repo,
        health,
    }
}
