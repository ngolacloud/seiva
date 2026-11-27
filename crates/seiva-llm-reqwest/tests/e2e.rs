//! A real HTTP round trip: `seiva-llm`'s codecs and router over reqwest, against
//! a local axum server that plays the Anthropic and OpenAI-compatible APIs.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router as Http};
use futures_util::StreamExt;
use seiva_llm::providers::anthropic::Anthropic;
use seiva_llm::providers::openai::OpenAiCompatible;
use seiva_llm::{
    ChatModel, ChatProvider, ChatRequest, Endpoint, LlmError, Residency, RetryPolicy, Route,
    Router, StaticCredentials, StreamEvent,
};
use seiva_llm_reqwest::ReqwestTransport;
use serde_json::{json, Value};

const KEY: &str = "sk-ant-e2e-secret-value";

#[derive(Clone, Default)]
struct Counters {
    busy: Arc<AtomicUsize>,
}

async fn messages(headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if headers.get("x-api-key").and_then(|v| v.to_str().ok()) != Some(KEY) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}}))).into_response();
    }
    if body["stream"] == true {
        let events = [
            json!({"type": "message_start", "message": {"model": body["model"], "usage": {"input_tokens": 5}}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Olá, "}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Luanda!"}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 3}}),
            json!({"type": "message_stop"}),
        ];
        let sse: String = events
            .iter()
            .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap()))
            .collect();
        return ([("content-type", "text/event-stream")], sse).into_response();
    }
    Json(json!({"model": body["model"], "content": [{"type": "text", "text": "Olá"}], "stop_reason": "end_turn", "usage": {"input_tokens": 5, "output_tokens": 1}}))
        .into_response()
}

async fn overloaded(State(c): State<Counters>) -> Response {
    c.busy.fetch_add(1, Ordering::SeqCst);
    (StatusCode::from_u16(529).unwrap(), Json(json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}))).into_response()
}

async fn redirect() -> Response {
    (StatusCode::FOUND, [("location", "/v1/messages")], "Found").into_response()
}

async fn chat_completions(Json(body): Json<Value>) -> Response {
    Json(json!({"model": body["model"], "choices": [{"message": {"content": "servido pelo local"}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 4, "completion_tokens": 3}}))
    .into_response()
}

async fn serve() -> (String, Counters) {
    let counters = Counters::default();
    let app = Http::new()
        .route("/v1/messages", post(messages))
        .route("/busy/v1/messages", post(overloaded))
        .route("/moved/v1/messages", post(redirect))
        .route("/local/v1/chat/completions", post(chat_completions))
        .with_state(counters.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), counters)
}

fn anthropic(base: &str, name: &str) -> Anthropic {
    Anthropic::new(
        Arc::new(ReqwestTransport::with_defaults().unwrap()),
        Arc::new(StaticCredentials::new().with("ANTHROPIC_API_KEY", KEY)),
    )
    .with_endpoint(Endpoint::allow_plaintext(base).unwrap())
    .named(name)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_whole_and_a_streamed_answer_over_real_http() {
    let (base, _) = serve().await;
    let p = anthropic(&base, "anthropic");
    let r = p
        .complete("claude-x", &ChatRequest::new(32).user("olá"))
        .await
        .unwrap();
    assert_eq!(r.text(), "Olá");
    assert_eq!(r.model, "claude-x");

    let events: Vec<_> = p
        .stream("claude-x", &ChatRequest::new(32).user("olá"))
        .await
        .unwrap()
        .collect()
        .await;
    match events.last() {
        Some(Ok(StreamEvent::Completed(r))) => {
            assert_eq!(r.text(), "Olá, Luanda!");
            assert_eq!((r.usage.input_tokens, r.usage.output_tokens), (5, 3));
        }
        other => panic!("unexpected end: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_key_is_a_401_and_the_key_is_not_in_the_error() {
    let (base, _) = serve().await;
    let p = Anthropic::new(
        Arc::new(ReqwestTransport::with_defaults().unwrap()),
        Arc::new(StaticCredentials::new().with("ANTHROPIC_API_KEY", "sk-wrong-key-123456")),
    )
    .with_endpoint(Endpoint::allow_plaintext(&base).unwrap());
    let err = p
        .complete("m", &ChatRequest::new(8).user("hi"))
        .await
        .unwrap_err();
    assert_eq!(err, LlmError::status(401, "invalid x-api-key"));
    assert!(!format!("{err:?}").contains("sk-wrong"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_redirect_is_returned_not_followed() {
    let (base, _) = serve().await;
    let p = anthropic(&format!("{base}/moved"), "anthropic");
    let err = p
        .complete("m", &ChatRequest::new(8).user("hi"))
        .await
        .unwrap_err();
    assert_eq!(err, LlmError::status(302, "Found"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_router_falls_back_from_an_overloaded_cloud_to_a_local_model() {
    let (base, counters) = serve().await;
    let local = OpenAiCompatible::new(
        "local",
        Endpoint::allow_plaintext(&format!("{base}/local/v1")).unwrap(),
        Arc::new(ReqwestTransport::with_defaults().unwrap()),
        Arc::new(StaticCredentials::new()),
    );
    let router = Router::builder()
        .provider(
            Arc::new(anthropic(&format!("{base}/busy"), "cloud")),
            Residency::Abroad,
        )
        .provider(Arc::new(local), Residency::InCountry)
        .route(
            Route::new("chat", Residency::Abroad)
                .requires("llm.chat")
                .upstream("cloud", "claude-x")
                .upstream("local", "llama"),
        )
        .retry(seiva_resilience_policy())
        .build()
        .unwrap();
    let r = router
        .route("chat")
        .unwrap()
        .complete(&ChatRequest::new(16).user("hi"))
        .await
        .unwrap();
    assert_eq!(r.text(), "servido pelo local");
    assert_eq!(r.upstream.as_deref(), Some("local/llama"));
    assert_eq!(
        counters.busy.load(Ordering::SeqCst),
        2,
        "529 is retried once, then the route falls back"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_listening_is_a_transport_error() {
    // Bind and drop: the port is free and refuses connections.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let p = anthropic(&format!("http://{addr}"), "anthropic");
    let err = p
        .complete("m", &ChatRequest::new(8).user("hi"))
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Transport(_)), "{err:?}");
    assert!(err.is_retryable());
}

fn seiva_resilience_policy() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 2,
        backoff_base_ms: 0,
    }
}
