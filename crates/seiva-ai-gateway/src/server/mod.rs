//! The server shape (ADR 0048 G3): the gateway behind OpenAI- and
//! Anthropic-compatible endpoints, so any client of either API — a corporate chat,
//! a RAG front end, Claude Code, an IDE — points its base URL here and inherits
//! every stage.
//!
//! | Endpoint | Shape |
//! |---|---|
//! | `POST /v1/chat/completions` | OpenAI Chat Completions, whole or streamed |
//! | `POST /v1/messages` | Anthropic Messages, whole or streamed |
//! | `POST /v1/messages/count_tokens` | Anthropic token count (an estimate) |
//! | `GET /v1/models` | the routes this caller may use, in a shape both clients read |
//!
//! The `model` a client sends is a gateway **route**. The client authenticates
//! with a gateway key (`Authorization: Bearer <key>` or `x-api-key: <key>`), never
//! a provider key; the key decides the principal and the area. Errors come back
//! in the client's own error shape, and never carry a prompt, an answer or a
//! value.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use futures_util::StreamExt;
use seiva_identity::AuthError;
use seiva_llm::{ChatRequest, LlmError};
use serde_json::{json, Value};

use crate::guard::request_chars;
use crate::{Caller, Gateway, GatewayError};

mod anthropic;
mod keys;
mod openai;

pub use keys::{hash_key, GatewayKeys, KeyEntry, KeyError};

/// Server limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerOptions {
    /// `max_tokens` when an OpenAI client sends none.
    pub default_max_tokens: u32,
    /// The largest `max_tokens` accepted; more is refused, not silently cut.
    pub max_tokens_cap: u32,
    /// The largest request body (images included).
    pub body_limit: usize,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            default_max_tokens: 4096,
            max_tokens_cap: 64_000,
            body_limit: 8 * 1024 * 1024,
        }
    }
}

/// What the endpoints share.
#[derive(Clone)]
pub struct ServerState {
    gateway: Gateway,
    keys: Arc<GatewayKeys>,
    options: ServerOptions,
}

impl ServerState {
    /// The gateway behind `keys`, with default limits.
    pub fn new(gateway: Gateway, keys: GatewayKeys) -> Self {
        Self {
            gateway,
            keys: Arc::new(keys),
            options: ServerOptions::default(),
        }
    }

    /// Other limits.
    #[must_use]
    pub fn with_options(mut self, options: ServerOptions) -> Self {
        self.options = options;
        self
    }
}

/// The endpoints, ready to nest in a `seiva-server` app or serve on their own.
pub fn router(state: ServerState) -> axum::Router {
    let limit = state.options.body_limit;
    axum::Router::new()
        .route("/v1/chat/completions", post(openai_chat))
        .route("/v1/messages", post(anthropic_messages))
        .route("/v1/messages/count_tokens", post(anthropic_count))
        .route("/v1/models", get(models))
        .layer(DefaultBodyLimit::max(limit))
        .with_state(state)
}

pub(crate) struct Limits {
    default_max_tokens: u32,
    cap: u32,
}

impl Limits {
    fn of(o: &ServerOptions) -> Self {
        Self {
            default_max_tokens: o.default_max_tokens,
            cap: o.max_tokens_cap,
        }
    }

    pub(crate) fn max_tokens(&self, requested: Option<u64>) -> Result<u32, String> {
        match requested {
            None => Ok(self.default_max_tokens.min(self.cap)),
            Some(0) => Err("`max_tokens` must be at least 1".into()),
            Some(n) if n > u64::from(self.cap) => Err(format!(
                "`max_tokens` {n} is above the gateway's cap of {}",
                self.cap
            )),
            Some(n) => Ok(n as u32),
        }
    }
}

pub(crate) struct Parsed {
    pub route: String,
    pub request: ChatRequest,
    pub stream: bool,
    pub include_usage: bool,
}

/// A `data:` URL as `(media type, base64)`. Remote URLs are refused: the gateway
/// does not fetch on a client's behalf.
pub(crate) fn data_url(url: &str) -> Result<(String, String), String> {
    let rest = url
        .strip_prefix("data:")
        .ok_or("only data: image URLs are accepted (the gateway does not fetch URLs)")?;
    let (meta, data) = rest.split_once(',').ok_or("a malformed data: URL")?;
    let media_type = meta
        .strip_suffix(";base64")
        .ok_or("a data: URL must be base64")?;
    Ok((media_type.to_owned(), data.to_owned()))
}

/// Tool arguments sent as a JSON string; text that is not JSON is kept as a
/// string for validation to reject.
pub(crate) fn arguments(raw: &str) -> Value {
    if raw.trim().is_empty() {
        return json!({});
    }
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    OpenAi,
    Anthropic,
}

struct ApiError {
    status: StatusCode,
    kind: &'static str,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            kind: "invalid_request_error",
            message: message.into(),
        }
    }

    fn from_auth(e: AuthError) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            kind: "authentication_error",
            message: match e {
                AuthError::MissingCredentials => {
                    "a gateway key is required (Authorization: Bearer, or x-api-key)".into()
                }
                _ => "the gateway key is not valid".into(),
            },
        }
    }

    fn from_gateway(e: &GatewayError) -> Self {
        let (status, kind) = match e {
            GatewayError::Unauthenticated => (StatusCode::UNAUTHORIZED, "authentication_error"),
            GatewayError::Forbidden { .. }
            | GatewayError::ToolNotAllowed { .. }
            | GatewayError::Residency { .. } => (StatusCode::FORBIDDEN, "permission_error"),
            GatewayError::BudgetExceeded { .. } => {
                (StatusCode::TOO_MANY_REQUESTS, "rate_limit_error")
            }
            GatewayError::Blocked { .. } => (StatusCode::BAD_REQUEST, "invalid_request_error"),
            GatewayError::Model(LlmError::UnknownRoute(_)) => {
                (StatusCode::NOT_FOUND, "not_found_error")
            }
            GatewayError::Model(_) => (StatusCode::BAD_GATEWAY, "api_error"),
            GatewayError::BudgetUnavailable(_) | GatewayError::GuardFailed { .. } => {
                (StatusCode::SERVICE_UNAVAILABLE, "api_error")
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "api_error"),
        };
        Self {
            status,
            kind,
            message: e.to_string(),
        }
    }

    fn render(&self, shape: Shape, request_id: &str) -> Response {
        let body = match shape {
            Shape::OpenAi => {
                json!({"error": {"message": self.message, "type": self.kind, "code": Value::Null}})
            }
            Shape::Anthropic => {
                json!({"type": "error", "error": {"type": self.kind, "message": self.message}})
            }
        };
        with_request_id((self.status, Json(body)).into_response(), request_id)
    }
}

fn with_request_id(mut response: Response, request_id: &str) -> Response {
    if let Ok(v) = HeaderValue::from_str(request_id) {
        response.headers_mut().insert("request-id", v);
    }
    response
}

fn next_id(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    format!(
        "{prefix}{nanos:016x}{:06x}",
        COUNTER.fetch_add(1, Ordering::Relaxed) & 0xff_ffff
    )
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// The client's request id, if it is a sane one, or a new one.
fn request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 128 && v.chars().all(|c| c.is_ascii_graphic()))
        .map_or_else(|| next_id("req_"), str::to_owned)
}

/// The caller behind the presented gateway key.
fn authenticate(
    state: &ServerState,
    headers: &HeaderMap,
    request_id: &str,
) -> Result<Caller, ApiError> {
    let key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| {
                    v.strip_prefix("Bearer ")
                        .or_else(|| v.strip_prefix("bearer "))
                })
        })
        .unwrap_or_default()
        .trim();
    let (principal, area) = state.keys.lookup(key).map_err(ApiError::from_auth)?;
    let mut caller = Caller::new(principal).with_request_id(request_id);
    if let Some(area) = area {
        caller = caller.in_area(area);
    }
    Ok(caller)
}

fn read_json(body: &Bytes) -> Result<Value, ApiError> {
    serde_json::from_slice(body)
        .map_err(|e| ApiError::bad_request(format!("the body is not JSON: {e}")))
}

async fn openai_chat(
    State(state): State<ServerState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let rid = request_id(&headers);
    let result: Result<Response, ApiError> = async {
        let caller = authenticate(&state, &headers, &rid)?;
        let parsed = openai::parse(&read_json(&body)?, &Limits::of(&state.options))
            .map_err(ApiError::bad_request)?;
        let id = next_id("chatcmpl-");
        let created = now_secs();
        if parsed.stream {
            let events = state
                .gateway
                .stream(&caller, &parsed.route, parsed.request)
                .await
                .map_err(|e| ApiError::from_gateway(&e))?;
            let mut chunker =
                openai::Chunker::new(&parsed.route, &id, created, parsed.include_usage);
            let sse = sse_from(events, move |item| match item {
                Ok(ev) => chunker
                    .on_event(&ev)
                    .into_iter()
                    .map(|d| (None, d))
                    .collect(),
                Err(e) => chunker
                    .on_error(&e.to_string())
                    .into_iter()
                    .map(|d| (None, d))
                    .collect(),
            });
            return Ok(sse);
        }
        let gated = state
            .gateway
            .complete(&caller, &parsed.route, parsed.request)
            .await
            .map_err(|e| ApiError::from_gateway(&e))?;
        Ok(Json(openai::render(&parsed.route, &id, created, &gated.response)).into_response())
    }
    .await;
    match result {
        Ok(r) => with_request_id(r, &rid),
        Err(e) => e.render(Shape::OpenAi, &rid),
    }
}

async fn anthropic_messages(
    State(state): State<ServerState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let rid = request_id(&headers);
    let result: Result<Response, ApiError> = async {
        let caller = authenticate(&state, &headers, &rid)?;
        let parsed = anthropic::parse(&read_json(&body)?, &Limits::of(&state.options))
            .map_err(ApiError::bad_request)?;
        let id = next_id("msg_");
        if parsed.stream {
            let events = state
                .gateway
                .stream(&caller, &parsed.route, parsed.request)
                .await
                .map_err(|e| ApiError::from_gateway(&e))?;
            let mut eventer = anthropic::Eventer::new(&parsed.route, &id);
            let sse = sse_from(events, move |item| match item {
                Ok(ev) => eventer
                    .on_event(&ev)
                    .into_iter()
                    .map(|(n, d)| (Some(n), d))
                    .collect(),
                Err(e) => eventer
                    .on_error(&e.to_string())
                    .into_iter()
                    .map(|(n, d)| (Some(n), d))
                    .collect(),
            });
            return Ok(sse);
        }
        let gated = state
            .gateway
            .complete(&caller, &parsed.route, parsed.request)
            .await
            .map_err(|e| ApiError::from_gateway(&e))?;
        Ok(Json(anthropic::render(&parsed.route, &id, &gated.response)).into_response())
    }
    .await;
    match result {
        Ok(r) => with_request_id(r, &rid),
        Err(e) => e.render(Shape::Anthropic, &rid),
    }
}

/// An estimate (characters ÷ 4): no model is called and nothing leaves.
async fn anthropic_count(
    State(state): State<ServerState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let rid = request_id(&headers);
    let result = (|| {
        authenticate(&state, &headers, &rid)?;
        let mut value = read_json(&body)?;
        // count_tokens carries no max_tokens; satisfy the parser.
        value["max_tokens"] = json!(1);
        let parsed =
            anthropic::parse(&value, &Limits::of(&state.options)).map_err(ApiError::bad_request)?;
        let tools: usize = parsed
            .request
            .tools
            .iter()
            .map(|t| t.description.len() + t.input_schema.to_string().len())
            .sum();
        let estimate = (request_chars(&parsed.request) + tools) / 4 + 1;
        Ok::<_, ApiError>(Json(json!({"input_tokens": estimate})).into_response())
    })();
    match result {
        Ok(r) => with_request_id(r, &rid),
        Err(e) => e.render(Shape::Anthropic, &rid),
    }
}

/// The routes this caller may use, in a shape both the OpenAI and the Anthropic
/// clients read (`object` for one, `type`/`display_name` for the other).
async fn models(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    let rid = request_id(&headers);
    let caller = match authenticate(&state, &headers, &rid) {
        Ok(c) => c,
        Err(e) => {
            let shape = if headers.contains_key("anthropic-version") {
                Shape::Anthropic
            } else {
                Shape::OpenAi
            };
            return e.render(shape, &rid);
        }
    };
    let names = state.gateway.routes_for(&caller);
    let data: Vec<Value> = names
        .iter()
        .map(|n| {
            json!({
                "id": n, "object": "model", "owned_by": "seiva-ai-gateway", "created": 0,
                "type": "model", "display_name": n, "created_at": "1970-01-01T00:00:00Z",
            })
        })
        .collect();
    let body = json!({
        "object": "list",
        "data": data,
        "has_more": false,
        "first_id": names.first(),
        "last_id": names.last(),
    });
    with_request_id(Json(body).into_response(), &rid)
}

/// A gateway event stream as server-sent events, each item mapped to
/// `(event name, data)` pairs. The stream ends after the final answer or an
/// error.
fn sse_from<F>(events: seiva_llm::EventStream, map: F) -> Response
where
    F: FnMut(Result<seiva_llm::StreamEvent, LlmError>) -> Vec<(Option<&'static str>, String)>
        + Send
        + 'static,
{
    struct St<F> {
        events: seiva_llm::EventStream,
        map: F,
        pending: VecDeque<Event>,
        done: bool,
    }
    let st = St {
        events,
        map,
        pending: VecDeque::new(),
        done: false,
    };
    let stream = futures_util::stream::unfold(st, |mut st| async move {
        loop {
            if let Some(e) = st.pending.pop_front() {
                return Some((Ok::<Event, Infallible>(e), st));
            }
            if st.done {
                return None;
            }
            match st.events.next().await {
                Some(item) => {
                    let end = matches!(item, Ok(seiva_llm::StreamEvent::Completed(_)) | Err(_));
                    for (name, data) in (st.map)(item) {
                        let ev = Event::default().data(data);
                        st.pending.push_back(match name {
                            Some(n) => ev.event(n),
                            None => ev,
                        });
                    }
                    if end {
                        st.done = true;
                    }
                }
                None => st.done = true,
            }
        }
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}
