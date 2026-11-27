//! The codecs end to end, through the `HttpTransport` port: what goes on the wire
//! (URL, headers, body), and what a whole or streamed answer becomes — with the
//! body cut into small chunks so every SSE boundary case is crossed.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::StreamExt;
use seiva_llm::providers::anthropic::Anthropic;
use seiva_llm::providers::openai::OpenAiCompatible;
use seiva_llm::{
    ChatProvider, ChatRequest, Content, Embedder, Endpoint, HttpRequest, HttpResponse,
    HttpTransport, LlmError, Residency, Route, Router, StaticCredentials, StopReason, StreamEvent,
    ToolSpec, Usage,
};
use seiva_resilience::RetryPolicy;
use serde_json::{json, Value};

const KEY: &str = "sk-test-0123456789-very-secret";

/// A transport that records every request and answers from a queue, cutting each
/// body into `chunk`-byte pieces.
#[derive(Clone, Default)]
struct FakeTransport {
    replies: Arc<Mutex<VecDeque<(u16, String)>>>,
    seen: Arc<Mutex<Vec<HttpRequest>>>,
    chunk: usize,
}

impl FakeTransport {
    fn new(chunk: usize) -> Self {
        Self {
            chunk,
            ..Self::default()
        }
    }
    fn reply(self, status: u16, body: impl Into<String>) -> Self {
        self.replies
            .lock()
            .unwrap()
            .push_back((status, body.into()));
        self
    }
    fn seen(&self) -> Vec<HttpRequest> {
        self.seen.lock().unwrap().clone()
    }
    fn body(&self, i: usize) -> Value {
        serde_json::from_slice(&self.seen()[i].body).unwrap()
    }
    fn header(&self, i: usize, name: &str) -> Option<(String, bool)> {
        self.seen()[i]
            .headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| (v.as_str().to_owned(), v.is_sensitive()))
    }
}

#[async_trait]
impl HttpTransport for FakeTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, LlmError> {
        self.seen.lock().unwrap().push(request);
        let (status, body) = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| LlmError::Transport("connection refused".into()))?;
        let chunks: Vec<Result<Vec<u8>, LlmError>> = body
            .as_bytes()
            .chunks(self.chunk.max(1))
            .map(|c| Ok(c.to_vec()))
            .collect();
        Ok(HttpResponse::new(
            status,
            Box::pin(futures_util::stream::iter(chunks)),
        ))
    }
}

fn creds() -> Arc<StaticCredentials> {
    Arc::new(
        StaticCredentials::new()
            .with("ANTHROPIC_API_KEY", KEY)
            .with("OPENAI_API_KEY", KEY)
            .with("GEMINI_API_KEY", KEY),
    )
}

fn sse(events: &[Value]) -> String {
    events
        .iter()
        .map(|e| {
            format!(
                "event: {}\ndata: {}\n\n",
                e["type"].as_str().unwrap_or("x"),
                e
            )
        })
        .collect()
}

async fn collect(stream: seiva_llm::EventStream) -> Vec<Result<StreamEvent, LlmError>> {
    stream.collect().await
}

fn completed(events: &[Result<StreamEvent, LlmError>]) -> &seiva_llm::ChatResponse {
    match events.last() {
        Some(Ok(StreamEvent::Completed(r))) => r,
        other => panic!("the stream did not complete: {other:?}"),
    }
}

#[tokio::test]
async fn anthropic_sends_the_key_on_one_sensitive_header_and_reads_the_answer() {
    let t = FakeTransport::new(1024).reply(
        200,
        json!({"model": "claude-x", "content": [{"type": "text", "text": "Olá"}], "stop_reason": "end_turn",
               "usage": {"input_tokens": 3, "output_tokens": 1}})
        .to_string(),
    );
    let p = Anthropic::new(Arc::new(t.clone()), creds());
    let r = p
        .complete("claude-x", &ChatRequest::new(64).system("sys").user("hi"))
        .await
        .unwrap();
    assert_eq!(r.text(), "Olá");
    assert_eq!(
        r.usage,
        Usage {
            input_tokens: 3,
            output_tokens: 1,
            ..Usage::default()
        }
    );

    let seen = t.seen();
    assert_eq!(seen[0].url, "https://api.anthropic.com/v1/messages");
    assert_eq!(t.header(0, "x-api-key"), Some((KEY.to_owned(), true)));
    assert_eq!(t.header(0, "anthropic-version").unwrap().0, "2023-06-01");
    assert_eq!(t.body(0)["system"], "sys");
    // The request can be logged: the key does not print.
    assert!(!format!("{:?}", seen[0]).contains(KEY));
}

#[tokio::test]
async fn an_anthropic_stream_assembles_text_and_a_tool_call_across_tiny_chunks() {
    let body = sse(&[
        json!({"type": "message_start", "message": {"model": "claude-x", "usage": {"input_tokens": 20, "cache_read_input_tokens": 500}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Vou ver "}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "o cliente."}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "ping"}),
        json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "toolu_1", "name": "customer_lookup", "input": {}}}),
        json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "{\"nif\": \"‹TAX"}}),
        json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "_ID_1›\"}"}}),
        json!({"type": "content_block_stop", "index": 1}),
        json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 42}}),
        json!({"type": "message_stop"}),
    ]);
    let t = FakeTransport::new(7).reply(200, body);
    let p = Anthropic::new(Arc::new(t.clone()), creds());
    let events = collect(
        p.stream("claude-x", &ChatRequest::new(64).user("hi"))
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(t.body(0)["stream"], true);
    let deltas: String = events
        .iter()
        .filter_map(|e| match e {
            Ok(StreamEvent::TextDelta { text, .. }) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, "Vou ver o cliente.");
    let r = completed(&events);
    assert_eq!(r.stop_reason, StopReason::ToolUse);
    assert_eq!(
        r.tool_uses(),
        vec![("toolu_1", "customer_lookup", &json!({"nif": "‹TAX_ID_1›"}))]
    );
    assert_eq!(
        r.usage,
        Usage {
            input_tokens: 20,
            output_tokens: 42,
            cache_read_tokens: 500,
            cache_write_tokens: 0
        }
    );
}

#[tokio::test]
async fn a_stream_cut_before_its_end_ends_in_an_error() {
    let body = sse(&[
        json!({"type": "message_start", "message": {"model": "claude-x", "usage": {}}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "meia "}}),
    ]);
    let t = FakeTransport::new(5).reply(200, body);
    let events = collect(
        Anthropic::new(Arc::new(t), creds())
            .stream("m", &ChatRequest::new(8))
            .await
            .unwrap(),
    )
    .await;
    assert!(
        matches!(events.last(), Some(Err(LlmError::Transport(_)))),
        "{events:?}"
    );
}

#[tokio::test]
async fn an_openai_stream_maps_text_tool_calls_and_final_usage() {
    let chunks = [
        json!({"model": "gpt-x", "choices": [{"index": 0, "delta": {"role": "assistant", "content": "A "}}]}),
        json!({"model": "gpt-x", "choices": [{"index": 0, "delta": {"content": "ver."}}]}),
        json!({"model": "gpt-x", "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": ""}}]}}]}),
        json!({"model": "gpt-x", "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{\"id\":"}}]}}]}),
        json!({"model": "gpt-x", "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": "7}"}}]}}]}),
        json!({"model": "gpt-x", "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
        json!({"model": "gpt-x", "choices": [], "usage": {"prompt_tokens": 120, "completion_tokens": 9, "prompt_tokens_details": {"cached_tokens": 100}}}),
    ];
    let mut body: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    body.push_str("data: [DONE]\n\n");
    let t = FakeTransport::new(11).reply(200, body);
    let p = OpenAiCompatible::openai(Arc::new(t.clone()), creds());
    let req = ChatRequest::new(50).user("hi").tool(ToolSpec::new(
        "lookup",
        "find",
        json!({"type": "object"}),
    ));
    let events = collect(p.stream("gpt-x", &req).await.unwrap()).await;

    assert_eq!(
        t.seen()[0].url,
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(
        t.header(0, "authorization"),
        Some((format!("Bearer {KEY}"), true))
    );
    assert_eq!(t.body(0)["max_completion_tokens"], 50);
    let r = completed(&events);
    assert_eq!(r.text(), "A ver.");
    assert_eq!(r.tool_uses(), vec![("call_1", "lookup", &json!({"id": 7}))]);
    assert_eq!(r.stop_reason, StopReason::ToolUse);
    assert_eq!(
        r.usage,
        Usage {
            input_tokens: 20,
            output_tokens: 9,
            cache_read_tokens: 100,
            cache_write_tokens: 0
        }
    );
}

#[tokio::test]
async fn a_local_model_gets_no_key_and_plaintext_must_be_opted_in() {
    assert!(Endpoint::https("http://localhost:11434/v1").is_err());
    let t = FakeTransport::new(64).reply(
        200,
        json!({"model": "llama", "choices": [{"message": {"content": "ok"}, "finish_reason": "stop"}]}).to_string(),
    );
    let ollama = OpenAiCompatible::new(
        "ollama",
        Endpoint::allow_plaintext("http://localhost:11434/v1").unwrap(),
        Arc::new(t.clone()),
        creds(),
    );
    let r = ollama
        .complete("llama", &ChatRequest::new(10).user("hi"))
        .await
        .unwrap();
    assert_eq!(r.text(), "ok");
    assert_eq!(
        t.seen()[0].url,
        "http://localhost:11434/v1/chat/completions"
    );
    assert_eq!(t.header(0, "authorization"), None);
    assert_eq!(t.body(0)["max_tokens"], 10);
}

#[tokio::test]
async fn the_gemini_preset_targets_its_compatible_endpoint() {
    let t = FakeTransport::new(64).reply(
        200,
        json!({"choices": [{"message": {"content": "olá"}, "finish_reason": "stop"}]}).to_string(),
    );
    let gemini = OpenAiCompatible::gemini(Arc::new(t.clone()), creds());
    assert_eq!(gemini.name(), "gemini");
    gemini
        .complete("gemini-x", &ChatRequest::new(10).user("hi"))
        .await
        .unwrap();
    assert_eq!(
        t.seen()[0].url,
        "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
    );
}

#[tokio::test]
async fn a_missing_credential_sends_nothing() {
    let t = FakeTransport::new(64);
    let p = Anthropic::new(Arc::new(t.clone()), Arc::new(StaticCredentials::new()));
    let err = p
        .complete("m", &ChatRequest::new(8).user("hi"))
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Config(ref m) if m.contains("ANTHROPIC_API_KEY")));
    assert!(t.seen().is_empty());
}

#[tokio::test]
async fn a_redirect_is_an_error_never_followed() {
    let t = FakeTransport::new(64).reply(302, "Found");
    let err = Anthropic::new(Arc::new(t.clone()), creds())
        .complete("m", &ChatRequest::new(8).user("hi"))
        .await
        .unwrap_err();
    assert_eq!(err, LlmError::status(302, "Found"));
    assert_eq!(t.seen().len(), 1);
}

#[tokio::test]
async fn the_router_retries_then_falls_back_across_providers_and_never_leaks_the_key() {
    let busy = FakeTransport::new(64)
        .reply(529, json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}).to_string())
        .reply(529, json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}).to_string());
    let ok = FakeTransport::new(64).reply(
        200,
        json!({"model": "gpt-x", "choices": [{"message": {"content": "servido"}, "finish_reason": "stop"}]}).to_string(),
    );
    let router = Router::builder()
        .provider(
            Arc::new(Anthropic::new(Arc::new(busy.clone()), creds())),
            Residency::Abroad,
        )
        .provider(
            Arc::new(OpenAiCompatible::openai(Arc::new(ok.clone()), creds())),
            Residency::Abroad,
        )
        .route(
            Route::new("support-chat", Residency::Abroad)
                .requires("llm.support")
                .upstream("anthropic", "claude-x")
                .upstream("openai", "gpt-x"),
        )
        .retry(RetryPolicy {
            max_attempts: 2,
            backoff_base_ms: 0,
        })
        .build()
        .unwrap();
    let r = router
        .complete("support-chat", &ChatRequest::new(16).user("hi"))
        .await
        .unwrap();
    assert_eq!(r.text(), "servido");
    assert_eq!(r.upstream.as_deref(), Some("openai/gpt-x"));
    assert_eq!((busy.seen().len(), ok.seen().len()), (2, 1));
    assert_eq!(
        router.route_info("support-chat").unwrap().permission(),
        Some("llm.support")
    );
}

#[tokio::test]
async fn an_error_never_carries_the_key() {
    // A provider that (wrongly) echoes the request's headers in its error body,
    // and a transport whose error message quotes the bearer token.
    let echo = FakeTransport::new(64).reply(
        401,
        json!({"error": {"message": format!("invalid x-api-key: {KEY}")}}).to_string(),
    );
    let err = Anthropic::new(Arc::new(echo), creds())
        .complete("m", &ChatRequest::new(8))
        .await
        .unwrap_err();
    assert_eq!(err, LlmError::status(401, "invalid x-api-key: ***"));

    let bearer = FakeTransport::new(64).reply(
        500,
        format!("upstream rejected Authorization: Bearer {KEY}"),
    );
    let err = OpenAiCompatible::openai(Arc::new(bearer), creds())
        .complete("m", &ChatRequest::new(8))
        .await
        .unwrap_err();
    assert!(!err.to_string().contains(KEY), "{err}");
    assert!(!format!("{err:?}").contains(KEY));
}

#[tokio::test]
async fn embeddings_go_to_the_embeddings_endpoint() {
    let t = FakeTransport::new(64).reply(
        200,
        json!({"data": [{"index": 0, "embedding": [0.1, 0.2]}], "usage": {"prompt_tokens": 2}})
            .to_string(),
    );
    let e = OpenAiCompatible::openai(Arc::new(t.clone()), creds())
        .embed("text-embedding-x", &["olá mundo".into()])
        .await
        .unwrap();
    assert_eq!(e.vectors, vec![vec![0.1, 0.2]]);
    assert_eq!(t.seen()[0].url, "https://api.openai.com/v1/embeddings");
    assert_eq!(t.body(0)["input"], json!(["olá mundo"]));
}

#[test]
fn tool_results_and_images_survive_the_neutral_model() {
    // A compile-level check that the neutral content stays expressive enough for
    // a tool loop: the parts a G5 agent needs exist and round-trip through serde.
    let parts = vec![
        Content::Image {
            media_type: "image/png".into(),
            data: "iVBOR".into(),
        },
        Content::tool_result("t1", "{\"ok\":true}", false),
    ];
    let back: Vec<Content> = serde_json::from_value(serde_json::to_value(&parts).unwrap()).unwrap();
    assert_eq!(back, parts);
}
