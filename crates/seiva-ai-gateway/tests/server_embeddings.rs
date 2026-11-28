//! `POST /v1/embeddings` (ADR 0048, G7): the OpenAI Embeddings shape over the
//! gateway's stages — and `seiva-llm`'s own OpenAI-compatible `Embedder`, pointed
//! at the gateway, working end to end against a fake upstream.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use seiva_ai_gateway::server::{hash_key, router, GatewayKeys, KeyEntry, ServerState};
use seiva_ai_gateway::{BudgetScope, Gateway, InMemoryBudget};
use seiva_identity::Principal;
use seiva_llm::providers::openai::OpenAiCompatible;
use seiva_llm::{
    Embedder, Endpoint, HttpRequest, HttpResponse, HttpTransport, LlmError, Residency, Route,
    RouteKind, Router, StaticCredentials,
};
use seiva_testing::llm::ScriptedModel;
use seiva_testing::{FixedClock, RecordingAuditSink};
use serde_json::{json, Value};
use tower::ServiceExt;

const INDEXER_KEY: &str = "sk-gw-indexer-3333333333333333";
const RITA_KEY: &str = "sk-gw-rita-4444444444444444";
const NIF: &str = "5417123456";

/// The fake upstream: an OpenAI-compatible embeddings server that records each
/// body it receives and answers a 3-d vector per input.
#[derive(Clone, Default)]
struct FakeUpstream {
    bodies: Arc<Mutex<Vec<Value>>>,
}

#[async_trait]
impl HttpTransport for FakeUpstream {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, LlmError> {
        assert!(request.url.ends_with("/embeddings"), "{}", request.url);
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let n = body["input"].as_array().map_or(1, Vec::len);
        self.bodies.lock().unwrap().push(body);
        let data: Vec<Value> = (0..n)
            .map(|i| json!({"object": "embedding", "index": i, "embedding": [0.5, i as f32, -1.0]}))
            .collect();
        let answer = json!({"object": "list", "data": data, "model": "text-embedding-3-small",
            "usage": {"prompt_tokens": 6 * n, "total_tokens": 6 * n}});
        Ok(HttpResponse::from_bytes(200, answer.to_string()))
    }
}

/// An `HttpTransport` that serves each request with the gateway's axum router,
/// in process — the gateway as a client sees it, minus the socket.
struct InProcess(axum::Router);

#[async_trait]
impl HttpTransport for InProcess {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, LlmError> {
        let path = request
            .url
            .strip_prefix("https://gateway.internal")
            .unwrap()
            .to_owned();
        let mut req = Request::builder().method("POST").uri(path);
        for (name, value) in &request.headers {
            req = req.header(name.as_str(), value.as_str());
        }
        let res = self
            .0
            .clone()
            .oneshot(req.body(Body::from(request.body)).unwrap())
            .await
            .unwrap();
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        Ok(HttpResponse::from_bytes(status, bytes.to_vec()))
    }
}

fn app() -> (axum::Router, FakeUpstream, RecordingAuditSink) {
    let upstream = FakeUpstream::default();
    let openai = Arc::new(
        OpenAiCompatible::new(
            "openai",
            Endpoint::https("https://api.upstream.test/v1").unwrap(),
            Arc::new(upstream.clone()),
            Arc::new(StaticCredentials::new().with("OPENAI_API_KEY", "sk-provider")),
        )
        .with_credential("OPENAI_API_KEY"),
    );
    let llm = Router::builder()
        .provider(openai.clone(), Residency::Abroad)
        .embedder("openai", openai, Residency::Abroad)
        .provider(
            Arc::new(ScriptedModel::new("local").reply_text("ok")),
            Residency::InCountry,
        )
        .route(
            Route::new("support-chat", Residency::Abroad)
                .requires("llm.support")
                .upstream("openai", "gpt"),
        )
        .route(
            Route::new("docs-embed", Residency::Abroad)
                .with_kind(RouteKind::Embedding)
                .requires("llm.embed")
                .upstream("openai", "text-embedding-3-small"),
        )
        .retry(seiva_llm::RetryPolicy {
            max_attempts: 1,
            backoff_base_ms: 0,
        })
        .build()
        .unwrap();
    let audit = RecordingAuditSink::new();
    let gateway = Gateway::builder(llm)
        .budget(Arc::new(
            InMemoryBudget::new(
                Arc::new(FixedClock::at_unix_secs(0)),
                Duration::from_secs(86_400),
            )
            .per(BudgetScope::Principal, 50_000),
        ))
        .audit(Arc::new(audit.clone()))
        .area_in_country("hr")
        .build()
        .unwrap();
    let keys = GatewayKeys::new()
        .with(KeyEntry {
            sha256: hash_key(INDEXER_KEY),
            principal: Principal::in_tenant("indexer", "acme", ["llm.embed", "llm.support"]),
            area: Some("support".into()),
        })
        .unwrap()
        .with(KeyEntry {
            sha256: hash_key(RITA_KEY),
            principal: Principal::new("rita", ["llm.embed"]),
            area: Some("hr".into()),
        })
        .unwrap();
    (router(ServerState::new(gateway, keys)), upstream, audit)
}

async fn post(app: &axum::Router, path: &str, key: Option<&str>, body: Value) -> (u16, Value) {
    let mut req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(k) = key {
        req = req.header("authorization", format!("Bearer {k}"));
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn openai_clients_get_vectors_and_the_upstream_never_sees_the_nif() {
    let (app, upstream, audit) = app();
    let (status, v) = post(
        &app,
        "/v1/embeddings",
        Some(INDEXER_KEY),
        json!({"model": "docs-embed", "input": [format!("NIF {NIF}"), "estorno"]}),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["object"], "list");
    assert_eq!(v["model"], "docs-embed");
    assert_eq!(v["data"][1]["index"], 1);
    assert_eq!(v["data"][1]["embedding"], json!([0.5, 1.0, -1.0]));
    assert_eq!(v["usage"]["prompt_tokens"], 12);
    let sent = upstream.bodies.lock().unwrap()[0].clone();
    // The route names the provider's model; the text arrives masked.
    assert_eq!(sent["model"], "text-embedding-3-small");
    assert_eq!(sent["input"], json!(["NIF ‹TAX_ID_1›", "estorno"]));
    let entry = &audit.entries()[0];
    assert_eq!(entry.operation, "llm.embed");
    assert!(!format!("{entry:?}").contains(NIF));
}

#[tokio::test]
async fn base64_vectors_for_the_official_sdks() {
    let (app, _, _) = app();
    let (status, v) = post(
        &app,
        "/v1/embeddings",
        Some(INDEXER_KEY),
        json!({"model": "docs-embed", "input": "olá", "encoding_format": "base64"}),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    // [0.5, 0.0, -1.0] as little-endian f32.
    assert_eq!(v["data"][0]["embedding"], "AAAAPwAAAAAAAIC/");
}

#[tokio::test]
async fn refusals_come_back_in_the_openai_error_shape() {
    let (app, upstream, _) = app();
    let cases = [
        (
            None,
            json!({"model": "docs-embed", "input": "a"}),
            401,
            "authentication_error",
        ),
        (
            Some(INDEXER_KEY),
            json!({"model": "support-chat", "input": "a"}),
            400,
            "invalid_request_error",
        ),
        (
            Some(INDEXER_KEY),
            json!({"model": "nope", "input": "a"}),
            404,
            "not_found_error",
        ),
        (
            Some(INDEXER_KEY),
            json!({"model": "docs-embed", "input": [1, 2]}),
            400,
            "invalid_request_error",
        ),
        (
            Some(RITA_KEY),
            json!({"model": "docs-embed", "input": "a"}),
            403,
            "permission_error",
        ),
        (
            Some(INDEXER_KEY),
            json!({"model": "docs-embed", "input": "token sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}),
            400,
            "invalid_request_error",
        ),
    ];
    for (key, body, status, kind) in cases {
        let (got, v) = post(&app, "/v1/embeddings", key, body.clone()).await;
        assert_eq!(
            (got, v["error"]["type"].as_str()),
            (status, Some(kind)),
            "{body} → {v}"
        );
    }
    // …and a chat call to an embedding route is refused the same way.
    let (got, v) = post(
        &app,
        "/v1/chat/completions",
        Some(INDEXER_KEY),
        json!({"model": "docs-embed", "messages": [{"role": "user", "content": "olá"}]}),
    )
    .await;
    assert_eq!(got, 400, "{v}");
    assert!(upstream.bodies.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_model_list_tells_embedding_routes_from_chat_routes() {
    let (app, _, _) = app();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .header("authorization", format!("Bearer {INDEXER_KEY}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    let caps: Vec<(String, Value)> = v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["id"].as_str().unwrap().to_owned(),
                m["capabilities"].clone(),
            )
        })
        .collect();
    assert_eq!(
        caps,
        vec![
            ("docs-embed".to_owned(), json!(["embeddings"])),
            ("support-chat".to_owned(), json!(["chat"])),
        ]
    );
}

#[tokio::test]
async fn seiva_llms_openai_embedder_works_against_the_gateway() {
    let (app, upstream, audit) = app();
    // A product points the OpenAI-compatible embedder at the gateway: the base URL
    // is the gateway, the key is a gateway key, the model is a route.
    let client = OpenAiCompatible::new(
        "gateway",
        Endpoint::https("https://gateway.internal/v1").unwrap(),
        Arc::new(InProcess(app)),
        Arc::new(StaticCredentials::new().with("GATEWAY_KEY", INDEXER_KEY)),
    )
    .with_credential("GATEWAY_KEY");
    let e = client
        .embed(
            "docs-embed",
            &[format!("O NIF {NIF} pagou."), "outra frase".into()],
        )
        .await
        .unwrap();
    assert_eq!(e.vectors, vec![vec![0.5, 0.0, -1.0], vec![0.5, 1.0, -1.0]]);
    assert_eq!(e.usage.input_tokens, 12);
    assert_eq!(
        upstream.bodies.lock().unwrap()[0]["input"],
        json!(["O NIF ‹TAX_ID_1› pagou.", "outra frase"])
    );
    assert_eq!(audit.entries()[0].actor, "indexer");
    // A refusal reaches the client as the gateway's status.
    let err = client
        .embed("support-chat", &["a".into()])
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Status { status: 400, .. }), "{err}");
}
