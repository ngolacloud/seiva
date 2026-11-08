//! The application layer's CQRS dispatch core: a command/query **mediator** with
//! cross-cutting **middleware**, decoupled from transport.
//!
//! A use case is a self-contained handler. A [`Command`] mutates state and a
//! [`Query`] reads it; a [`CommandHandler`] or [`QueryHandler`] carries out one.
//! The [`Mediator`] routes a message to its handler and `await`s it, so a caller
//! (an HTTP controller, a gRPC service, a worker) depends on the mediator and
//! never on the concrete handler. This crate is **DORM-free**: it knows only the
//! result/error boundary from `seiva-ports`.
//!
//! # Static dispatch, not a runtime registry
//!
//! Dispatch here is **statically typed and zero-cost**. [`Mediator`] is generic
//! over the handler it holds (`Mediator::new(handler)`); `send`/`query` resolve
//! to a direct call at compile time with no boxing and no `TypeId` lookup. This
//! is deliberate and matches Seiva's "everything known at compile time"
//! principle: a missing handler is a *type error*, not a runtime panic, and the
//! optimiser can inline the whole chain. Rust has no reflection, so a global
//! `TypeId → Box<dyn Any>` map would (a) reintroduce a runtime failure mode the
//! type system otherwise rules out, (b) force boxing and dynamic dispatch, and
//! (c) make middleware composition untyped. Handlers are therefore **injected**
//! (constructor / application state), not registered.
//!
//! # Middleware
//!
//! A middleware is itself a handler that wraps an inner handler, so concerns
//! compose by nesting and stay fully typed. Two are shipped to prove the seam:
//! [`Instrument`] counts dispatched messages (the hook `#[policy]` instrumentation
//! will map onto), and [`CacheQueries`] memoises one read-side [`Query`] (the hook
//! `#[cached]` will map onto). Wrap from the inside out:
//! `Mediator::new(Instrument::new(CacheQueries::new(handler)))`.

#![forbid(unsafe_code)]
// This is a framework, not a `dyn`-dispatched object: handlers are always used
// generically (the [`Mediator`] is monomorphised over the concrete handler), so
// the returned future never needs boxing. `async fn` in these traits is exactly
// the ergonomic, zero-cost form we want; `seiva-ports`' `UnitOfWork` allows it
// for the same reason. We require `+ Send` on the returned future explicitly so
// handlers stay usable across `.await` points on a multi-threaded executor.
#![allow(async_fn_in_trait)]

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use seiva_ports::PortResult;

/// A message that *changes* state. Its [`Output`](Command::Output) is what the
/// handler returns on success (an id, a DTO, or `()`), carried back through the
/// [`Mediator`]. `Send` so a command can cross an `.await` on a work-stealing
/// executor.
pub trait Command: Send {
    /// What dispatching this command yields on success.
    type Output: Send;
}

/// A message that *reads* state without changing it. Its
/// [`Output`](Query::Output) is the value read. `Send` so a query can cross an
/// `.await`. Because a query has no side effects, its result is safe to memoise
/// (see [`CacheQueries`]) — unlike a [`Command`], which must never be cached.
pub trait Query: Send {
    /// What dispatching this query yields on success.
    type Output: Send;
}

/// Carries out one [`Command`] type `C`.
///
/// A handler is a use case: injected with the ports it needs and invoked through
/// the [`Mediator`]. The returned future is `+ Send` so the handler composes on a
/// multi-threaded runtime; an error surfaces as [`PortResult`]`::Err`.
pub trait CommandHandler<C: Command>: Send + Sync {
    /// Handle `cmd`, returning its [`Output`](Command::Output) or an error.
    fn handle(&self, cmd: C) -> impl Future<Output = PortResult<C::Output>> + Send;
}

/// Carries out one [`Query`] type `Q`.
///
/// The read-side twin of [`CommandHandler`]: same shape, but for a side-effect-free
/// read whose result may be memoised by [`CacheQueries`].
pub trait QueryHandler<Q: Query>: Send + Sync {
    /// Handle `query`, returning its [`Output`](Query::Output) or an error.
    fn handle(&self, query: Q) -> impl Future<Output = PortResult<Q::Output>> + Send;
}

/// The dispatch entrypoint: routes a message to the handler it holds.
///
/// Generic over the handler `H` (or a middleware chain wrapping it), so dispatch
/// is a direct, monomorphised call — see the [crate docs](crate) for why this is
/// preferred over a runtime registry. Construct with the handler injected, then
/// [`send`](Mediator::send) a command or run a [`query`](Mediator::query).
///
/// ```
/// use seiva_cqrs::{Command, CommandHandler, Mediator};
/// use seiva_ports::PortResult;
///
/// // A command and the use case that carries it out.
/// struct Add(u64, u64);
/// impl Command for Add {
///     type Output = u64;
/// }
///
/// struct Calculator;
/// impl CommandHandler<Add> for Calculator {
///     async fn handle(&self, cmd: Add) -> PortResult<u64> {
///         Ok(cmd.0 + cmd.1)
///     }
/// }
///
/// // Wire the handler into a mediator and dispatch. (A tiny current-thread
/// // runtime stands in for the application's executor.)
/// let mediator = Mediator::new(Calculator);
/// let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
/// let sum = rt.block_on(mediator.send(Add(2, 3))).unwrap();
/// assert_eq!(sum, 5);
/// ```
pub struct Mediator<H> {
    handler: H,
}

impl<H> Mediator<H> {
    /// Wrap a handler (or a middleware chain) so messages can be dispatched to it.
    pub fn new(handler: H) -> Self {
        Self { handler }
    }

    /// Dispatch `cmd` to the held [`CommandHandler`] and await its result.
    pub async fn send<C>(&self, cmd: C) -> PortResult<C::Output>
    where
        C: Command,
        H: CommandHandler<C>,
    {
        self.handler.handle(cmd).await
    }

    /// Dispatch `query` to the held [`QueryHandler`] and await its result.
    pub async fn query<Q>(&self, query: Q) -> PortResult<Q::Output>
    where
        Q: Query,
        H: QueryHandler<Q>,
    {
        self.handler.handle(query).await
    }

    /// Borrow the wrapped handler (or middleware), e.g. to read a middleware's
    /// metrics after dispatch.
    pub fn handler(&self) -> &H {
        &self.handler
    }
}

/// Instrumentation middleware: counts every message dispatched through it.
///
/// It wraps an inner handler and implements both [`CommandHandler`] and
/// [`QueryHandler`] by bumping a shared counter and delegating. This is the seam
/// the facade's `#[policy]` instrumentation maps onto; a production build can add
/// a `tracing` span around the same delegation. The counter is an `Arc`, so a
/// clone taken before the handler is moved into a [`Mediator`] observes the same
/// total — or read it back through [`Mediator::handler`] and [`dispatched`].
///
/// [`dispatched`]: Instrument::dispatched
pub struct Instrument<H> {
    inner: H,
    dispatched: Arc<AtomicU64>,
}

impl<H> Instrument<H> {
    /// Wrap `inner`, starting the dispatch count at zero.
    pub fn new(inner: H) -> Self {
        Self {
            inner,
            dispatched: Arc::new(AtomicU64::new(0)),
        }
    }

    /// How many messages (commands and queries) have been dispatched so far.
    pub fn dispatched(&self) -> u64 {
        self.dispatched.load(Ordering::SeqCst)
    }

    /// A shared handle to the dispatch counter, so a caller can keep observing
    /// the total after the middleware is moved into a [`Mediator`].
    pub fn counter(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.dispatched)
    }

    /// Borrow the wrapped inner handler/middleware.
    pub fn inner(&self) -> &H {
        &self.inner
    }
}

impl<C, H> CommandHandler<C> for Instrument<H>
where
    C: Command,
    H: CommandHandler<C>,
{
    async fn handle(&self, cmd: C) -> PortResult<C::Output> {
        self.dispatched.fetch_add(1, Ordering::SeqCst);
        self.inner.handle(cmd).await
    }
}

impl<Q, H> QueryHandler<Q> for Instrument<H>
where
    Q: Query,
    H: QueryHandler<Q>,
{
    async fn handle(&self, query: Q) -> PortResult<Q::Output> {
        self.dispatched.fetch_add(1, Ordering::SeqCst);
        self.inner.handle(query).await
    }
}

/// Read-through caching middleware for a single [`Query`] type `Q`.
///
/// Memoises `Q`'s result by the query value itself as the key, so a repeated
/// identical query is served from the cache and the inner handler runs **once**
/// per distinct key. The design is fully typed — one `CacheQueries<H, Q>` caches
/// exactly one query type, so there is no `TypeId`/`Any` map — and it is the seam
/// the facade's `#[cached]` maps onto.
///
/// Bounds, and why:
/// - `Q: Eq + Hash` — the query is its own cache key, so identical queries hit.
/// - `Q: Clone` — the key must be stored alongside the result.
/// - `Q::Output: Clone` — a cached value is handed out by clone to each caller.
///
/// **Commands are never cached.** A write has side effects, so caching it would
/// be incorrect; this middleware also implements [`CommandHandler`] as a plain
/// pass-through, so a command placed in the same chain always reaches its
/// handler. Only reads ([`QueryHandler`]) are memoised.
///
/// # Security: the query key must carry every authorization discriminant
///
/// The cache key **is the query value**. In a multi-tenant service the query must
/// therefore include the caller's tenant (and any other scope that changes the
/// result), exactly as a correct CQRS read already does — the query is the
/// complete description of *what is being read, for whom*. A query that omits the
/// tenant (e.g. `GetNote { id }` when the result differs per tenant) is already a
/// correctness bug, and caching turns it into a **cross-tenant disclosure**: one
/// tenant's result served to another under the same key (OWASP API1). Build the
/// query from `Ctx::tenant` server-side; never cache a query whose key does not
/// pin the scope.
///
/// The cache is also **unbounded** (one entry per distinct key); apply it only to
/// read types with a bounded key space, or front it with a size/TTL policy, so a
/// client-controlled filter space cannot grow it without limit (OWASP API4).
pub struct CacheQueries<H, Q>
where
    Q: Query + Eq + Hash,
{
    inner: H,
    cache: Mutex<HashMap<Q, Q::Output>>,
}

impl<H, Q> CacheQueries<H, Q>
where
    Q: Query + Eq + Hash,
{
    /// Wrap `inner` with an empty cache.
    pub fn new(inner: H) -> Self {
        Self {
            inner,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// How many distinct query keys are currently memoised.
    pub fn len(&self) -> usize {
        self.cache.lock().expect("cache mutex poisoned").len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Borrow the wrapped inner handler/middleware.
    pub fn inner(&self) -> &H {
        &self.inner
    }
}

impl<H, Q> QueryHandler<Q> for CacheQueries<H, Q>
where
    Q: Query + Eq + Hash + Clone,
    Q::Output: Clone,
    H: QueryHandler<Q>,
{
    async fn handle(&self, query: Q) -> PortResult<Q::Output> {
        // Look up under the lock, then release it *before* awaiting: the mutex is
        // never held across an `.await`, so the returned future stays `Send`.
        {
            let cache = self.cache.lock().expect("cache mutex poisoned");
            if let Some(hit) = cache.get(&query) {
                return Ok(hit.clone());
            }
        }
        let output = self.inner.handle(query.clone()).await?;
        self.cache
            .lock()
            .expect("cache mutex poisoned")
            .insert(query, output.clone());
        Ok(output)
    }
}

impl<C, H, Q> CommandHandler<C> for CacheQueries<H, Q>
where
    C: Command,
    H: CommandHandler<C>,
    Q: Query + Eq + Hash,
{
    async fn handle(&self, cmd: C) -> PortResult<C::Output> {
        // A command is never cached — it is forwarded untouched every time.
        self.inner.handle(cmd).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- In-memory command side -------------------------------------------

    struct Sum(u64, u64);
    impl Command for Sum {
        type Output = u64;
    }

    struct Adder;
    impl CommandHandler<Sum> for Adder {
        async fn handle(&self, cmd: Sum) -> PortResult<u64> {
            Ok(cmd.0 + cmd.1)
        }
    }

    struct Boom;
    impl Command for Boom {
        type Output = ();
    }

    struct Failing;
    impl CommandHandler<Boom> for Failing {
        async fn handle(&self, _cmd: Boom) -> PortResult<()> {
            Err("handler blew up".into())
        }
    }

    // ----- In-memory query side ----------------------------------------------

    #[derive(Clone, PartialEq, Eq, Hash)]
    struct GetGreeting(u64);
    impl Query for GetGreeting {
        type Output = String;
    }

    /// A query handler that records how many times it actually ran, so a test can
    /// prove the cache suppressed a repeat call.
    struct Greeter {
        calls: Arc<AtomicU64>,
    }
    impl Greeter {
        fn new() -> Self {
            Self {
                calls: Arc::new(AtomicU64::new(0)),
            }
        }
        fn calls(&self) -> Arc<AtomicU64> {
            Arc::clone(&self.calls)
        }
    }
    impl QueryHandler<GetGreeting> for Greeter {
        async fn handle(&self, query: GetGreeting) -> PortResult<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(format!("hello #{}", query.0))
        }
    }

    // ----- Tests --------------------------------------------------------------

    #[tokio::test]
    async fn a_command_runs_and_returns_its_output() {
        let mediator = Mediator::new(Adder);
        let out = mediator.send(Sum(2, 40)).await.unwrap();
        assert_eq!(out, 42);
    }

    #[tokio::test]
    async fn a_query_runs_and_returns_its_output() {
        let mediator = Mediator::new(Greeter::new());
        let out = mediator.query(GetGreeting(7)).await.unwrap();
        assert_eq!(out, "hello #7");
    }

    #[tokio::test]
    async fn a_handler_error_propagates_as_err() {
        let mediator = Mediator::new(Failing);
        let result = mediator.send(Boom).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "handler blew up");
    }

    #[tokio::test]
    async fn the_cache_serves_the_second_identical_query_from_cache() {
        let handler = Greeter::new();
        let calls = handler.calls();
        let mediator = Mediator::new(CacheQueries::new(handler));

        let first = mediator.query(GetGreeting(1)).await.unwrap();
        let second = mediator.query(GetGreeting(1)).await.unwrap();

        assert_eq!(first, "hello #1");
        assert_eq!(second, first);
        // The inner handler ran exactly once; the second query was a cache hit.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(mediator.handler().len(), 1);
    }

    #[tokio::test]
    async fn distinct_query_keys_each_miss_the_cache() {
        let handler = Greeter::new();
        let calls = handler.calls();
        let mediator = Mediator::new(CacheQueries::new(handler));

        let a = mediator.query(GetGreeting(1)).await.unwrap();
        let b = mediator.query(GetGreeting(2)).await.unwrap();

        assert_eq!(a, "hello #1");
        assert_eq!(b, "hello #2");
        // Two distinct keys ran the handler twice and are cached separately.
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(mediator.handler().len(), 2);
    }

    #[tokio::test]
    async fn the_logging_middleware_records_each_dispatch() {
        let mediator = Mediator::new(Instrument::new(Adder));

        assert_eq!(mediator.handler().dispatched(), 0);
        mediator.send(Sum(1, 1)).await.unwrap();
        mediator.send(Sum(2, 2)).await.unwrap();

        assert_eq!(mediator.handler().dispatched(), 2);
    }

    #[tokio::test]
    async fn middleware_composes_instrument_over_cache() {
        let handler = Greeter::new();
        let calls = handler.calls();
        // Instrument on the outside counts every dispatch; the cache on the
        // inside suppresses the repeat handler call.
        let mediator = Mediator::new(Instrument::new(CacheQueries::new(handler)));

        mediator.query(GetGreeting(9)).await.unwrap();
        mediator.query(GetGreeting(9)).await.unwrap();

        // Two dispatches observed, but the handler ran only once.
        assert_eq!(mediator.handler().dispatched(), 2);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_command_passes_through_the_cache_layer_untouched() {
        // A command routed through a query cache must always reach its handler.
        let mediator = Mediator::new(CacheQueries::<_, GetGreeting>::new(Adder));
        let a = mediator.send(Sum(10, 5)).await.unwrap();
        let b = mediator.send(Sum(10, 5)).await.unwrap();
        assert_eq!(a, 15);
        assert_eq!(b, 15);
        // Nothing was cached: the cache only ever memoises its query type.
        assert!(mediator.handler().is_empty());
    }
}
