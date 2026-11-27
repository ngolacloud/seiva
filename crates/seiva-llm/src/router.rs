//! Routes: what a product calls instead of a provider's model id.
//!
//! A [`Route`] names its upstreams (provider + model) in fallback order, the data
//! [`Residency`] it promises, and the permission a caller must hold. The
//! [`Router`] checks the whole table when it is built — an unknown provider, an
//! `in_country` route with an `abroad` upstream, a route with no access policy
//! are configuration errors, not surprises at request time — and then serves each
//! call: every attempt has a time budget, transient failures are retried with the
//! framework-wide backoff (`seiva-resilience`), and an upstream that keeps failing
//! hands over to the next one.
//!
//! The router does **not** check the caller's permission: that is the
//! gateway's `Authorize` stage (ADR 0048 §2), which reads
//! [`Route::permission`]. The router only guarantees the route has one.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use seiva_resilience::{timeout, RetryPolicy};

use crate::stream::EventStream;
use crate::{ChatModel, ChatProvider, ChatRequest, ChatResponse, LlmError, StreamEvent};

/// Where a provider processes data, and where a route lets data go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Residency {
    /// Inside the country (the organisation's own models, or a provider region
    /// in the country). An `in_country` route only ever reaches these.
    InCountry,
    /// Outside the country. Masking is mandatory on such a route (ADR 0048 §2).
    Abroad,
}

/// One provider and model a route may use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    /// The provider's [`ChatProvider::name`].
    pub provider: String,
    /// The model id at that provider.
    pub model: String,
}

impl std::fmt::Display for Upstream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.provider, self.model)
    }
}

/// A named route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    name: String,
    residency: Residency,
    permission: Option<String>,
    public: bool,
    upstreams: Vec<Upstream>,
}

impl Route {
    /// A route with no upstreams and no access policy yet: add both, or the
    /// router refuses it (fail-closed, as a facade method without `#[requires]`
    /// is a compile error — ADR 0019).
    pub fn new(name: impl Into<String>, residency: Residency) -> Self {
        Self {
            name: name.into(),
            residency,
            permission: None,
            public: false,
            upstreams: Vec::new(),
        }
    }

    /// The permission a caller must hold to use the route.
    #[must_use]
    pub fn requires(mut self, permission: impl Into<String>) -> Self {
        self.permission = Some(permission.into());
        self
    }

    /// Any authenticated caller may use the route — an explicit, auditable choice,
    /// like `#[public]` on a facade method.
    #[must_use]
    pub fn public(mut self) -> Self {
        self.public = true;
        self
    }

    /// Add an upstream; the order is the fallback order.
    #[must_use]
    pub fn upstream(mut self, provider: impl Into<String>, model: impl Into<String>) -> Self {
        self.upstreams.push(Upstream {
            provider: provider.into(),
            model: model.into(),
        });
        self
    }

    /// The route name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Where the route lets data go.
    pub fn residency(&self) -> Residency {
        self.residency
    }

    /// The permission a caller must hold, or `None` for a public route.
    pub fn permission(&self) -> Option<&str> {
        self.permission.as_deref()
    }

    /// Whether the route was declared public.
    pub fn is_public(&self) -> bool {
        self.public
    }

    /// The upstreams, in fallback order.
    pub fn upstreams(&self) -> &[Upstream] {
        &self.upstreams
    }
}

struct Registered {
    residency: Residency,
    provider: Arc<dyn ChatProvider>,
}

struct Inner {
    providers: HashMap<String, Registered>,
    routes: HashMap<String, Route>,
    retry: RetryPolicy,
    attempt_timeout: Duration,
}

/// Serves routes. Cheap to clone (one shared table).
#[derive(Clone)]
pub struct Router {
    inner: Arc<Inner>,
}

/// Builds a [`Router`].
pub struct RouterBuilder {
    providers: Vec<(Residency, Arc<dyn ChatProvider>)>,
    routes: Vec<Route>,
    retry: RetryPolicy,
    attempt_timeout: Duration,
}

impl Default for RouterBuilder {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            routes: Vec::new(),
            // Model calls are slow and rate limits need room: three attempts,
            // half a second doubling.
            retry: RetryPolicy {
                max_attempts: 3,
                backoff_base_ms: 500,
            },
            attempt_timeout: Duration::from_secs(120),
        }
    }
}

impl RouterBuilder {
    /// Register a provider under its [`ChatProvider::name`], declaring where it
    /// processes data.
    #[must_use]
    pub fn provider(mut self, provider: Arc<dyn ChatProvider>, residency: Residency) -> Self {
        self.providers.push((residency, provider));
        self
    }

    /// Add a route.
    #[must_use]
    pub fn route(mut self, route: Route) -> Self {
        self.routes.push(route);
        self
    }

    /// The retry policy of each upstream.
    #[must_use]
    pub fn retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// The time budget of one attempt (and, while streaming, of the wait for the
    /// next event).
    #[must_use]
    pub fn attempt_timeout(mut self, budget: Duration) -> Self {
        self.attempt_timeout = budget;
        self
    }

    /// Check the table and build the router.
    pub fn build(self) -> Result<Router, LlmError> {
        let mut providers = HashMap::new();
        for (residency, provider) in self.providers {
            let name = provider.name().to_owned();
            if providers
                .insert(
                    name.clone(),
                    Registered {
                        residency,
                        provider,
                    },
                )
                .is_some()
            {
                return Err(LlmError::Config(format!(
                    "provider `{name}` is registered twice"
                )));
            }
        }
        let mut routes = HashMap::new();
        for route in self.routes {
            let name = route.name.clone();
            match (&route.permission, route.public) {
                (Some(p), false) if !p.trim().is_empty() => {}
                (None, true) => {}
                (Some(_), true) => return Err(LlmError::Config(format!("route `{name}` is both public and permission-guarded"))),
                _ => {
                    return Err(LlmError::Config(format!(
                        "route `{name}` declares no access policy: add `.requires(\"permission\")` or `.public()`"
                    )))
                }
            }
            if route.upstreams.is_empty() {
                return Err(LlmError::Config(format!("route `{name}` has no upstream")));
            }
            for up in &route.upstreams {
                let Some(registered) = providers.get(&up.provider) else {
                    return Err(LlmError::Config(format!(
                        "route `{name}`: unknown provider `{}`",
                        up.provider
                    )));
                };
                if route.residency == Residency::InCountry
                    && registered.residency == Residency::Abroad
                {
                    return Err(LlmError::Config(format!(
                        "route `{name}` keeps data in the country, but upstream `{up}` is abroad"
                    )));
                }
            }
            if routes.insert(name.clone(), route).is_some() {
                return Err(LlmError::Config(format!("route `{name}` is defined twice")));
            }
        }
        Ok(Router {
            inner: Arc::new(Inner {
                providers,
                routes,
                retry: self.retry,
                attempt_timeout: self.attempt_timeout,
            }),
        })
    }
}

impl Router {
    /// Start a table.
    pub fn builder() -> RouterBuilder {
        RouterBuilder::default()
    }

    /// The route called `name`, for the gateway's checks.
    pub fn route_info(&self, name: &str) -> Option<&Route> {
        self.inner.routes.get(name)
    }

    /// The route called `name` as a [`ChatModel`] — what a use case holds.
    pub fn route(&self, name: &str) -> Result<RouteHandle, LlmError> {
        if !self.inner.routes.contains_key(name) {
            return Err(LlmError::UnknownRoute(name.to_owned()));
        }
        Ok(RouteHandle {
            router: self.clone(),
            route: name.to_owned(),
        })
    }

    fn lookup(&self, route: &str) -> Result<&Route, LlmError> {
        self.inner
            .routes
            .get(route)
            .ok_or_else(|| LlmError::UnknownRoute(route.to_owned()))
    }

    fn provider(&self, up: &Upstream) -> &Arc<dyn ChatProvider> {
        // The builder proved every upstream's provider exists.
        &self.inner.providers[&up.provider].provider
    }

    /// A whole answer from the route.
    pub async fn complete(
        &self,
        route: &str,
        request: &ChatRequest,
    ) -> Result<ChatResponse, LlmError> {
        let r = self.lookup(route)?;
        let budget = self.inner.attempt_timeout;
        let mut last = None;
        for up in &r.upstreams {
            let provider = self.provider(up);
            let outcome = self
                .inner
                .retry
                .retry_if(
                    || async {
                        timeout(budget, provider.complete(&up.model, request))
                            .await
                            .unwrap_or(Err(LlmError::Timeout))
                    },
                    LlmError::is_retryable,
                )
                .await;
            match outcome {
                Ok(mut response) => {
                    response.upstream = Some(up.to_string());
                    tracing::debug!(route, upstream = %up, input = response.usage.total_input(), output = response.usage.output_tokens, "model call served");
                    return Ok(response);
                }
                Err(e) => {
                    tracing::warn!(route, upstream = %up, error = %e, "upstream failed");
                    if !e.allows_fallback() {
                        return Err(e);
                    }
                    last = Some(e);
                }
            }
        }
        Err(LlmError::AllUpstreamsFailed {
            route: route.to_owned(),
            last: Box::new(last.unwrap_or(LlmError::Config("no upstream".into()))),
        })
    }

    /// A streamed answer from the route. Fallback happens only before the first
    /// event: once text has reached the caller, an error ends the stream.
    pub async fn stream(
        &self,
        route: &str,
        request: &ChatRequest,
    ) -> Result<EventStream, LlmError> {
        let r = self.lookup(route)?;
        let budget = self.inner.attempt_timeout;
        let mut last = None;
        for up in &r.upstreams {
            let provider = self.provider(up);
            let outcome = self
                .inner
                .retry
                .retry_if(
                    || async {
                        timeout(budget, provider.stream(&up.model, request))
                            .await
                            .unwrap_or(Err(LlmError::Timeout))
                    },
                    LlmError::is_retryable,
                )
                .await;
            match outcome {
                Ok(stream) => return Ok(guard_stream(stream, budget, up.to_string())),
                Err(e) => {
                    tracing::warn!(route, upstream = %up, error = %e, "upstream failed");
                    if !e.allows_fallback() {
                        return Err(e);
                    }
                    last = Some(e);
                }
            }
        }
        Err(LlmError::AllUpstreamsFailed {
            route: route.to_owned(),
            last: Box::new(last.unwrap_or(LlmError::Config("no upstream".into()))),
        })
    }
}

/// Bound the wait for each event by `budget`, stamp the upstream on the final
/// answer, and end the stream after its first error.
fn guard_stream(stream: EventStream, budget: Duration, upstream: String) -> EventStream {
    Box::pin(futures_util::stream::unfold(Some(stream), move |state| {
        let upstream = upstream.clone();
        async move {
            let mut stream = state?;
            match timeout(budget, stream.next()).await {
                Err(_) => Some((Err(LlmError::Timeout), None)),
                Ok(None) => None,
                Ok(Some(Err(e))) => Some((Err(e), None)),
                Ok(Some(Ok(StreamEvent::Completed(mut response)))) => {
                    response.upstream = Some(upstream);
                    Some((Ok(StreamEvent::Completed(response)), Some(stream)))
                }
                Ok(Some(Ok(event))) => Some((Ok(event), Some(stream))),
            }
        }
    }))
}

/// One route of a [`Router`], as a [`ChatModel`].
#[derive(Clone)]
pub struct RouteHandle {
    router: Router,
    route: String,
}

impl RouteHandle {
    /// The route's name.
    pub fn name(&self) -> &str {
        &self.route
    }
}

#[async_trait]
impl ChatModel for RouteHandle {
    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, LlmError> {
        self.router.complete(&self.route, request).await
    }

    async fn stream(&self, request: &ChatRequest) -> Result<EventStream, LlmError> {
        self.router.stream(&self.route, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Content, StopReason, Usage};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// A provider that plays a script of outcomes and counts its calls.
    struct Scripted {
        name: &'static str,
        script: Mutex<VecDeque<Result<ChatResponse, LlmError>>>,
        calls: Mutex<u32>,
    }

    impl Scripted {
        fn new(name: &'static str, script: Vec<Result<ChatResponse, LlmError>>) -> Arc<Self> {
            Arc::new(Self {
                name,
                script: Mutex::new(script.into()),
                calls: Mutex::new(0),
            })
        }
        fn calls(&self) -> u32 {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl ChatProvider for Scripted {
        fn name(&self) -> &str {
            self.name
        }
        async fn complete(&self, model: &str, _: &ChatRequest) -> Result<ChatResponse, LlmError> {
            *self.calls.lock().unwrap() += 1;
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(answer(model)))
        }
    }

    fn answer(model: &str) -> ChatResponse {
        ChatResponse::new(
            model,
            vec![Content::text("ok")],
            StopReason::EndTurn,
            Usage::default(),
        )
    }

    fn fast() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 2,
            backoff_base_ms: 0,
        }
    }

    #[test]
    fn a_route_without_an_access_policy_is_refused() {
        let p = Scripted::new("p", vec![]);
        let err = Router::builder()
            .provider(p, Residency::Abroad)
            .route(Route::new("chat", Residency::Abroad).upstream("p", "m"))
            .build()
            .err()
            .unwrap();
        assert!(matches!(err, LlmError::Config(m) if m.contains("declares no access policy")));
    }

    #[test]
    fn an_in_country_route_cannot_reach_an_abroad_provider() {
        let local = Scripted::new("local", vec![]);
        let cloud = Scripted::new("cloud", vec![]);
        let err = Router::builder()
            .provider(local, Residency::InCountry)
            .provider(cloud, Residency::Abroad)
            .route(
                Route::new("hr", Residency::InCountry)
                    .requires("llm.hr")
                    .upstream("local", "llama")
                    .upstream("cloud", "big"),
            )
            .build()
            .err()
            .unwrap();
        assert!(matches!(err, LlmError::Config(m) if m.contains("upstream `cloud/big` is abroad")));
    }

    #[test]
    fn unknown_providers_and_empty_routes_are_refused() {
        let build = |route: Route| {
            Router::builder()
                .provider(Scripted::new("p", vec![]), Residency::Abroad)
                .route(route)
                .build()
        };
        assert!(build(
            Route::new("a", Residency::Abroad)
                .public()
                .upstream("nope", "m")
        )
        .is_err());
        assert!(build(Route::new("a", Residency::Abroad).public()).is_err());
        assert!(build(
            Route::new("a", Residency::Abroad)
                .requires("  ")
                .upstream("p", "m")
        )
        .is_err());
    }

    #[tokio::test]
    async fn a_transient_failure_is_retried_on_the_same_upstream() {
        let p = Scripted::new("p", vec![Err(LlmError::status(429, "slow down"))]);
        let router = Router::builder()
            .provider(p.clone(), Residency::Abroad)
            .route(
                Route::new("chat", Residency::Abroad)
                    .public()
                    .upstream("p", "m"),
            )
            .retry(fast())
            .build()
            .unwrap();
        let r = router
            .complete("chat", &ChatRequest::new(10).user("hi"))
            .await
            .unwrap();
        assert_eq!(p.calls(), 2);
        assert_eq!(r.upstream.as_deref(), Some("p/m"));
    }

    #[tokio::test]
    async fn a_failing_upstream_hands_over_to_the_next() {
        let a = Scripted::new("a", vec![Err(LlmError::status(401, "bad key"))]);
        let b = Scripted::new("b", vec![]);
        let router = Router::builder()
            .provider(a.clone(), Residency::Abroad)
            .provider(b.clone(), Residency::Abroad)
            .route(
                Route::new("chat", Residency::Abroad)
                    .requires("llm.chat")
                    .upstream("a", "m1")
                    .upstream("b", "m2"),
            )
            .retry(fast())
            .build()
            .unwrap();
        let r = router
            .route("chat")
            .unwrap()
            .complete(&ChatRequest::new(10).user("hi"))
            .await
            .unwrap();
        // 401 is not retried on `a`, but does fall back to `b`.
        assert_eq!((a.calls(), b.calls()), (1, 1));
        assert_eq!(r.upstream.as_deref(), Some("b/m2"));
    }

    #[tokio::test]
    async fn a_bad_request_stops_the_route() {
        let a = Scripted::new("a", vec![Err(LlmError::status(400, "messages: empty"))]);
        let b = Scripted::new("b", vec![]);
        let router = Router::builder()
            .provider(a, Residency::Abroad)
            .provider(b.clone(), Residency::Abroad)
            .route(
                Route::new("chat", Residency::Abroad)
                    .public()
                    .upstream("a", "m1")
                    .upstream("b", "m2"),
            )
            .retry(fast())
            .build()
            .unwrap();
        let err = router
            .complete("chat", &ChatRequest::new(10))
            .await
            .unwrap_err();
        assert_eq!(err, LlmError::status(400, "messages: empty"));
        assert_eq!(b.calls(), 0);
    }

    #[tokio::test]
    async fn when_every_upstream_fails_the_last_error_is_kept() {
        let a = Scripted::new(
            "a",
            vec![
                Err(LlmError::status(503, "down")),
                Err(LlmError::status(503, "down")),
            ],
        );
        let router = Router::builder()
            .provider(a, Residency::Abroad)
            .route(
                Route::new("chat", Residency::Abroad)
                    .public()
                    .upstream("a", "m1"),
            )
            .retry(fast())
            .build()
            .unwrap();
        let err = router
            .complete("chat", &ChatRequest::new(10))
            .await
            .unwrap_err();
        assert!(
            matches!(err, LlmError::AllUpstreamsFailed { ref route, ref last } if route == "chat" && **last == LlmError::status(503, "down"))
        );
    }

    #[tokio::test]
    async fn an_unknown_route_is_an_error() {
        let router = Router::builder().build().unwrap();
        assert_eq!(
            router.route("x").err(),
            Some(LlmError::UnknownRoute("x".into()))
        );
        assert!(router.complete("x", &ChatRequest::new(1)).await.is_err());
    }

    struct Slow;

    #[async_trait]
    impl ChatProvider for Slow {
        fn name(&self) -> &str {
            "slow"
        }
        async fn complete(&self, model: &str, _: &ChatRequest) -> Result<ChatResponse, LlmError> {
            tokio::time::sleep(Duration::from_secs(600)).await;
            Ok(answer(model))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_attempt_that_outlives_its_budget_times_out() {
        let router = Router::builder()
            .provider(Arc::new(Slow), Residency::Abroad)
            .route(
                Route::new("chat", Residency::Abroad)
                    .public()
                    .upstream("slow", "m"),
            )
            .retry(RetryPolicy {
                max_attempts: 1,
                backoff_base_ms: 0,
            })
            .attempt_timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let err = router
            .complete("chat", &ChatRequest::new(10))
            .await
            .unwrap_err();
        assert!(
            matches!(err, LlmError::AllUpstreamsFailed { last, .. } if *last == LlmError::Timeout)
        );
    }

    #[tokio::test]
    async fn a_streamed_answer_is_stamped_with_its_upstream() {
        let p = Scripted::new("p", vec![]);
        let router = Router::builder()
            .provider(p, Residency::InCountry)
            .route(
                Route::new("chat", Residency::InCountry)
                    .public()
                    .upstream("p", "m"),
            )
            .build()
            .unwrap();
        let events: Vec<_> = router
            .stream("chat", &ChatRequest::new(10))
            .await
            .unwrap()
            .collect()
            .await;
        match events.last().unwrap() {
            Ok(StreamEvent::Completed(r)) => assert_eq!(r.upstream.as_deref(), Some("p/m")),
            other => panic!("unexpected {other:?}"),
        }
    }
}
