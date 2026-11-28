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
//! A route serves one [`RouteKind`] — chat by default, or embeddings — and a call
//! of the other kind is refused ([`LlmError::WrongRouteKind`]): a chat route is not
//! an embedding route unless its configuration says
//! [`RouteKind::ChatAndEmbedding`]. An embedding route's upstreams are providers
//! registered with [`RouterBuilder::embedder`]; the same residency rule holds.
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
use crate::{
    ChatModel, ChatProvider, ChatRequest, ChatResponse, Embedder, Embeddings, LlmError, StreamEvent,
};

/// Where a provider processes data, and where a route lets data go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Residency {
    /// Inside the country (the organisation's own models, or a provider region
    /// in the country). An `in_country` route only ever reaches these.
    InCountry,
    /// Outside the country. Masking is mandatory on such a route (ADR 0048 §2).
    Abroad,
}

/// What a route serves. A route is one kind unless its configuration says both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum RouteKind {
    /// Chat completions (the default).
    #[default]
    Chat,
    /// Embeddings: text in, vectors out.
    Embedding,
    /// Both — an explicit choice, so a chat route never turns into an embedding
    /// route (or the other way round) by accident.
    ChatAndEmbedding,
}

impl RouteKind {
    /// Whether the route answers chat calls.
    pub fn serves_chat(self) -> bool {
        matches!(self, Self::Chat | Self::ChatAndEmbedding)
    }

    /// Whether the route answers embedding calls.
    pub fn serves_embeddings(self) -> bool {
        matches!(self, Self::Embedding | Self::ChatAndEmbedding)
    }

    /// `chat`, `embedding` or `chat+embedding` — the spelling of configuration
    /// files.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Embedding => "embedding",
            Self::ChatAndEmbedding => "chat+embedding",
        }
    }

    /// Read [`as_str`](Self::as_str)'s spelling.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "chat" => Some(Self::Chat),
            "embedding" => Some(Self::Embedding),
            "chat+embedding" => Some(Self::ChatAndEmbedding),
            _ => None,
        }
    }
}

impl std::fmt::Display for RouteKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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
    kind: RouteKind,
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
            kind: RouteKind::Chat,
            permission: None,
            public: false,
            upstreams: Vec::new(),
        }
    }

    /// What the route serves (chat by default).
    #[must_use]
    pub fn with_kind(mut self, kind: RouteKind) -> Self {
        self.kind = kind;
        self
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

    /// What the route serves.
    pub fn kind(&self) -> RouteKind {
        self.kind
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
    chat: Option<Arc<dyn ChatProvider>>,
    embedder: Option<Arc<dyn Embedder>>,
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
    embedders: Vec<(String, Residency, Arc<dyn Embedder>)>,
    routes: Vec<Route>,
    retry: RetryPolicy,
    attempt_timeout: Duration,
}

impl Default for RouterBuilder {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            embedders: Vec::new(),
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

    /// Register an embedder under `name`, declaring where it processes data. A
    /// provider that serves both (an OpenAI-compatible server) is registered twice
    /// under the same name — with [`provider`](Self::provider) and here — and the
    /// two residencies must agree.
    #[must_use]
    pub fn embedder(
        mut self,
        name: impl Into<String>,
        embedder: Arc<dyn Embedder>,
        residency: Residency,
    ) -> Self {
        self.embedders.push((name.into(), residency, embedder));
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
        let mut providers: HashMap<String, Registered> = HashMap::new();
        for (residency, provider) in self.providers {
            let name = provider.name().to_owned();
            if providers
                .insert(
                    name.clone(),
                    Registered {
                        residency,
                        chat: Some(provider),
                        embedder: None,
                    },
                )
                .is_some()
            {
                return Err(LlmError::Config(format!(
                    "provider `{name}` is registered twice"
                )));
            }
        }
        for (name, residency, embedder) in self.embedders {
            match providers.get_mut(&name) {
                Some(r) if r.embedder.is_some() => {
                    return Err(LlmError::Config(format!(
                        "embedder `{name}` is registered twice"
                    )))
                }
                Some(r) if r.residency != residency => {
                    return Err(LlmError::Config(format!(
                        "provider `{name}` is registered with two residencies"
                    )))
                }
                Some(r) => r.embedder = Some(embedder),
                None => {
                    providers.insert(
                        name,
                        Registered {
                            residency,
                            chat: None,
                            embedder: Some(embedder),
                        },
                    );
                }
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
                if route.kind.serves_chat() && registered.chat.is_none() {
                    return Err(LlmError::Config(format!(
                        "route `{name}` serves chat, but provider `{}` has no chat",
                        up.provider
                    )));
                }
                if route.kind.serves_embeddings() && registered.embedder.is_none() {
                    return Err(LlmError::Config(format!(
                        "route `{name}` serves embeddings, but provider `{}` has no embeddings",
                        up.provider
                    )));
                }
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

    /// Every route, by name.
    pub fn routes(&self) -> Vec<&Route> {
        let mut routes: Vec<&Route> = self.inner.routes.values().collect();
        routes.sort_by(|a, b| a.name.cmp(&b.name));
        routes
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

    fn lookup(&self, route: &str, wanted: RouteKind) -> Result<&Route, LlmError> {
        let r = self
            .inner
            .routes
            .get(route)
            .ok_or_else(|| LlmError::UnknownRoute(route.to_owned()))?;
        let serves = match wanted {
            RouteKind::Embedding => r.kind.serves_embeddings(),
            _ => r.kind.serves_chat(),
        };
        if serves {
            Ok(r)
        } else {
            Err(LlmError::WrongRouteKind {
                route: route.to_owned(),
                wanted,
            })
        }
    }

    // The builder proved every upstream's provider exists and serves the route's
    // kind; the `Config` errors below are unreachable through `build`.
    fn provider(&self, up: &Upstream) -> Result<&Arc<dyn ChatProvider>, LlmError> {
        self.inner
            .providers
            .get(&up.provider)
            .and_then(|r| r.chat.as_ref())
            .ok_or_else(|| LlmError::Config(format!("provider `{}` has no chat", up.provider)))
    }

    fn embedder(&self, up: &Upstream) -> Result<&Arc<dyn Embedder>, LlmError> {
        self.inner
            .providers
            .get(&up.provider)
            .and_then(|r| r.embedder.as_ref())
            .ok_or_else(|| {
                LlmError::Config(format!("provider `{}` has no embeddings", up.provider))
            })
    }

    /// A whole answer from the route.
    pub async fn complete(
        &self,
        route: &str,
        request: &ChatRequest,
    ) -> Result<ChatResponse, LlmError> {
        let r = self.lookup(route, RouteKind::Chat)?;
        let budget = self.inner.attempt_timeout;
        let mut last = None;
        for up in &r.upstreams {
            let provider = self.provider(up)?;
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
        let r = self.lookup(route, RouteKind::Chat)?;
        let budget = self.inner.attempt_timeout;
        let mut last = None;
        for up in &r.upstreams {
            let provider = self.provider(up)?;
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

impl Router {
    /// Vectors from an embedding route: one per input, in order. The same retry,
    /// time budget and fallback as a chat call; an answer with the wrong number of
    /// vectors counts as a failed upstream.
    pub async fn embed(&self, route: &str, inputs: &[String]) -> Result<Embeddings, LlmError> {
        self.embed_route(route, inputs).await
    }

    async fn embed_route(&self, route: &str, inputs: &[String]) -> Result<Embeddings, LlmError> {
        let r = self.lookup(route, RouteKind::Embedding)?;
        let budget = self.inner.attempt_timeout;
        let mut last = None;
        for up in &r.upstreams {
            let embedder = self.embedder(up)?;
            let outcome = self
                .inner
                .retry
                .retry_if(
                    || async {
                        let e = timeout(budget, embedder.embed(&up.model, inputs))
                            .await
                            .unwrap_or(Err(LlmError::Timeout))?;
                        if e.vectors.len() == inputs.len() {
                            Ok(e)
                        } else {
                            Err(LlmError::Decode(format!(
                                "expected {} embeddings, got {}",
                                inputs.len(),
                                e.vectors.len()
                            )))
                        }
                    },
                    LlmError::is_retryable,
                )
                .await;
            match outcome {
                Ok(mut embeddings) => {
                    embeddings.upstream = Some(up.to_string());
                    tracing::debug!(route, upstream = %up, inputs = inputs.len(), input = embeddings.usage.total_input(), "embedding call served");
                    return Ok(embeddings);
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
}

/// A router is an [`Embedder`] whose `model` is an embedding **route** — so an
/// ingestor or a retriever holds the router and names the route, never a provider
/// model.
#[async_trait]
impl Embedder for Router {
    async fn embed(&self, model: &str, inputs: &[String]) -> Result<Embeddings, LlmError> {
        self.embed_route(model, inputs).await
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

    /// An embedder that fails `fail` times, then answers one 2-d vector per input
    /// (or `short` vectors when set).
    struct Vectors {
        fail: Mutex<u32>,
        short: bool,
        calls: Mutex<u32>,
    }

    impl Vectors {
        fn new(fail: u32) -> Arc<Self> {
            Arc::new(Self {
                fail: Mutex::new(fail),
                short: false,
                calls: Mutex::new(0),
            })
        }
        fn calls(&self) -> u32 {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl Embedder for Vectors {
        async fn embed(&self, _: &str, inputs: &[String]) -> Result<Embeddings, LlmError> {
            *self.calls.lock().unwrap() += 1;
            let mut fail = self.fail.lock().unwrap();
            if *fail > 0 {
                *fail -= 1;
                return Err(LlmError::status(503, "down"));
            }
            let n = if self.short { 0 } else { inputs.len() };
            Ok(Embeddings::new(
                vec![vec![1.0, 0.0]; n],
                Usage {
                    input_tokens: 3,
                    ..Usage::default()
                },
            ))
        }
    }

    #[test]
    fn route_kinds_read_and_print_their_configuration_spelling() {
        for k in [
            RouteKind::Chat,
            RouteKind::Embedding,
            RouteKind::ChatAndEmbedding,
        ] {
            assert_eq!(RouteKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(RouteKind::parse("embeddings"), None);
        assert!(RouteKind::ChatAndEmbedding.serves_chat());
        assert!(!RouteKind::Embedding.serves_chat() && !RouteKind::Chat.serves_embeddings());
    }

    #[test]
    fn an_embedding_route_needs_providers_that_embed() {
        let chat_only = Scripted::new("p", vec![]);
        let err = Router::builder()
            .provider(chat_only, Residency::Abroad)
            .route(
                Route::new("embed", Residency::Abroad)
                    .with_kind(RouteKind::Embedding)
                    .public()
                    .upstream("p", "m"),
            )
            .build()
            .err()
            .unwrap();
        assert!(matches!(err, LlmError::Config(m) if m.contains("has no embeddings")));
        // …and a chat route needs providers that chat.
        let err = Router::builder()
            .embedder("e", Vectors::new(0), Residency::Abroad)
            .route(
                Route::new("chat", Residency::Abroad)
                    .public()
                    .upstream("e", "m"),
            )
            .build()
            .err()
            .unwrap();
        assert!(matches!(err, LlmError::Config(m) if m.contains("has no chat")));
    }

    #[test]
    fn an_embedder_keeps_its_providers_residency() {
        let err = Router::builder()
            .provider(Scripted::new("p", vec![]), Residency::Abroad)
            .embedder("p", Vectors::new(0), Residency::InCountry)
            .build()
            .err()
            .unwrap();
        assert!(matches!(err, LlmError::Config(m) if m.contains("two residencies")));
        let err = Router::builder()
            .embedder("cloud", Vectors::new(0), Residency::Abroad)
            .route(
                Route::new("hr-embed", Residency::InCountry)
                    .with_kind(RouteKind::Embedding)
                    .requires("llm.hr")
                    .upstream("cloud", "e"),
            )
            .build()
            .err()
            .unwrap();
        assert!(matches!(err, LlmError::Config(m) if m.contains("is abroad")));
    }

    #[tokio::test]
    async fn a_route_serves_only_its_kind() {
        let p = Scripted::new("p", vec![]);
        let router = Router::builder()
            .provider(p.clone(), Residency::Abroad)
            .embedder("p", Vectors::new(0), Residency::Abroad)
            .route(
                Route::new("chat", Residency::Abroad)
                    .public()
                    .upstream("p", "m"),
            )
            .route(
                Route::new("embed", Residency::Abroad)
                    .with_kind(RouteKind::Embedding)
                    .public()
                    .upstream("p", "e"),
            )
            .route(
                Route::new("both", Residency::Abroad)
                    .with_kind(RouteKind::ChatAndEmbedding)
                    .public()
                    .upstream("p", "m"),
            )
            .build()
            .unwrap();
        let err = router.embed("chat", &["x".into()]).await.unwrap_err();
        assert_eq!(
            err,
            LlmError::WrongRouteKind {
                route: "chat".into(),
                wanted: RouteKind::Embedding
            }
        );
        let err = router
            .complete("embed", &ChatRequest::new(10))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            LlmError::WrongRouteKind {
                wanted: RouteKind::Chat,
                ..
            }
        ));
        assert!(router.stream("embed", &ChatRequest::new(10)).await.is_err());
        assert_eq!(p.calls(), 0);
        assert!(router.embed("both", &["x".into()]).await.is_ok());
        assert!(router.complete("both", &ChatRequest::new(10)).await.is_ok());
    }

    #[tokio::test]
    async fn an_embedding_route_retries_falls_back_and_stamps_its_upstream() {
        let a = Vectors::new(5);
        let b = Vectors::new(0);
        let router = Router::builder()
            .embedder("a", a.clone(), Residency::InCountry)
            .embedder("b", b.clone(), Residency::InCountry)
            .route(
                Route::new("embed", Residency::InCountry)
                    .with_kind(RouteKind::Embedding)
                    .requires("llm.embed")
                    .upstream("a", "e1")
                    .upstream("b", "e2"),
            )
            .retry(fast())
            .build()
            .unwrap();
        // The router is an `Embedder` whose model is the route.
        let embedder: &dyn Embedder = &router;
        let e = embedder
            .embed("embed", &["um".into(), "dois".into()])
            .await
            .unwrap();
        assert_eq!((a.calls(), b.calls()), (2, 1));
        assert_eq!(e.vectors.len(), 2);
        assert_eq!(e.upstream.as_deref(), Some("b/e2"));
    }

    #[tokio::test]
    async fn the_wrong_number_of_vectors_is_a_failed_upstream() {
        let short = Arc::new(Vectors {
            fail: Mutex::new(0),
            short: true,
            calls: Mutex::new(0),
        });
        let router = Router::builder()
            .embedder("a", short, Residency::Abroad)
            .route(
                Route::new("embed", Residency::Abroad)
                    .with_kind(RouteKind::Embedding)
                    .public()
                    .upstream("a", "e"),
            )
            .retry(fast())
            .build()
            .unwrap();
        let err = router.embed("embed", &["x".into()]).await.unwrap_err();
        assert!(
            matches!(err, LlmError::AllUpstreamsFailed { last, .. } if matches!(*last, LlmError::Decode(_)))
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
