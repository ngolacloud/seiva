//! The runnable gateway end to end over real HTTP: a fake upstream plays the
//! Anthropic and OpenAI-compatible APIs, the gateway is built from a TOML file
//! exactly as `main` builds it, and real clients of both shapes call it.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use ai_gateway::{build, entry_json, Config, SystemClock};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Json;
use seiva_ai_gateway::server::hash_key;
use seiva_llm::StaticCredentials;
use seiva_llm_reqwest::ReqwestTransport;
use seiva_testing::RecordingAuditSink;
use serde_json::{json, Value};

const PROVIDER_KEY: &str = "sk-ant-provider-0000000000000000";
const GATEWAY_KEY: &str = "sk-gw-desk-3333333333333333";
const NIF: &str = "5417123456";

/// What the fake upstream saw: (path, headers as text, body).
type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

async fn anthropic_up(
    State(seen): State<Seen>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    seen.lock()
        .unwrap()
        .push(("anthropic".into(), format!("{headers:?}"), body.clone()));
    if headers.get("x-api-key").and_then(|v| v.to_str().ok()) != Some(PROVIDER_KEY) {
        return (axum::http::StatusCode::UNAUTHORIZED, Json(json!({"type": "error", "error": {"type": "authentication_error", "message": "bad key"}})))
            .into_response();
    }
    // Echo back the placeholder the model received.
    let asked = body["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let placeholder = asked
        .split_whitespace()
        .find(|w| w.starts_with('‹'))
        .unwrap_or("?")
        .trim_end_matches('?')
        .to_owned();
    Json(json!({"model": body["model"], "content": [{"type": "text", "text": format!("Confirmo o NIF {placeholder}.")}],
                "stop_reason": "end_turn", "usage": {"input_tokens": 12, "output_tokens": 5}}))
    .into_response()
}

async fn openai_up(
    State(seen): State<Seen>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    seen.lock()
        .unwrap()
        .push(("openai".into(), format!("{headers:?}"), body.clone()));
    if body["stream"] == true {
        let chunks = [
            json!({"model": "llama", "choices": [{"index": 0, "delta": {"content": "Olá "}}]}),
            json!({"model": "llama", "choices": [{"index": 0, "delta": {"content": "do servidor local."}, "finish_reason": "stop"}]}),
            json!({"model": "llama", "choices": [], "usage": {"prompt_tokens": 7, "completion_tokens": 4}}),
        ];
        let mut sse: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
        sse.push_str("data: [DONE]\n\n");
        return ([("content-type", "text/event-stream")], sse).into_response();
    }
    Json(json!({"model": "llama", "choices": [{"message": {"content": "local"}, "finish_reason": "stop"}]})).into_response()
}

async fn spawn(app: axum::Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

struct Running {
    gateway: String,
    seen: Seen,
    audit: RecordingAuditSink,
    health: seiva_http::Health,
}

async fn start() -> Running {
    let seen: Seen = Arc::default();
    let upstream = spawn(
        axum::Router::new()
            .route("/v1/messages", post(anthropic_up))
            .route("/local/v1/chat/completions", post(openai_up))
            .with_state(seen.clone()),
    )
    .await;
    let toml = format!(
        r#"
listen = "127.0.0.1:0"

[budget]
period_secs = 86400
per_principal = 100000

[[providers]]
name = "cloud"
kind = "anthropic"
residency = "abroad"
endpoint = "http://{upstream}"
plaintext = true

[[providers]]
name = "local"
kind = "openai-compatible"
residency = "in_country"
endpoint = "http://{upstream}/local/v1"
plaintext = true

[[routes]]
name = "support-chat"
residency = "abroad"
requires = "llm.support"
upstreams = ["cloud/claude-x"]

[[routes]]
name = "internal"
residency = "in_country"
public = true
upstreams = ["local/llama"]

[gateway]
areas_in_country = ["hr"]

[[keys]]
subject = "support-desk"
sha256 = "{hash}"
permissions = ["llm.support"]
tenant = "acme"
area = "support"
"#,
        hash = hash_key(GATEWAY_KEY)
    );
    let config = Config::parse(&toml).unwrap();
    let audit = RecordingAuditSink::new();
    let health = seiva_http::Health::starting();
    let app = build(
        &config,
        Arc::new(ReqwestTransport::with_defaults().unwrap()),
        Arc::new(StaticCredentials::new().with("ANTHROPIC_API_KEY", PROVIDER_KEY)),
        Arc::new(audit.clone()),
        Arc::new(SystemClock),
        health.clone(),
    )
    .unwrap();
    let addr = spawn(app).await;
    Running {
        gateway: format!("http://{addr}"),
        seen,
        audit,
        health,
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_openai_client_reaches_an_anthropic_model_through_the_gateway() {
    let g = start().await;
    let res = client()
        .post(format!("{}/v1/chat/completions", g.gateway))
        .header("authorization", format!("Bearer {GATEWAY_KEY}"))
        .header("content-type", "application/json")
        .body(json!({"model": "support-chat", "messages": [{"role": "user", "content": format!("O NIF {NIF} está registado?")}]}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let v: Value = serde_json::from_str(&res.text().await.unwrap()).unwrap();
    assert_eq!(
        v["choices"][0]["message"]["content"],
        format!("Confirmo o NIF {NIF}.")
    );

    let seen = g.seen.lock().unwrap().clone();
    let (path, headers, body) = &seen[0];
    assert_eq!(path, "anthropic");
    // The provider got the organisation's key and a placeholder; never the
    // gateway key, never the NIF.
    assert!(headers.contains("x-api-key"));
    assert!(!headers.contains(GATEWAY_KEY), "{headers}");
    assert!(!body.to_string().contains(NIF), "{body}");
    assert!(body.to_string().contains("‹TAX_ID_1›"));
    assert_eq!(body["model"], "claude-x");

    let entry = &g.audit.entries()[0];
    let line = entry_json(entry).to_string();
    assert!(
        line.contains("\"actor\":\"support-desk\"") && line.contains("\"outcome\":\"ok\""),
        "{line}"
    );
    assert!(!line.contains(NIF) && !line.contains(GATEWAY_KEY));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_anthropic_client_streams_from_a_local_model() {
    let g = start().await;
    let res = client()
        .post(format!("{}/v1/messages", g.gateway))
        .header("x-api-key", GATEWAY_KEY)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(json!({"model": "internal", "max_tokens": 64, "stream": true, "messages": [{"role": "user", "content": "olá"}]}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    let text: String = body
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|d| serde_json::from_str::<Value>(d).ok())
        .filter_map(|v| v["delta"]["text"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(text, "Olá do servidor local.");
    assert!(body.contains("event: message_stop"), "{body}");
    // The local model got no key at all.
    let seen = g.seen.lock().unwrap().clone();
    assert!(!seen[0].1.contains("authorization"), "{}", seen[0].1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_key_never_reaches_a_provider_and_health_is_served() {
    let g = start().await;
    let res = client()
        .post(format!("{}/v1/messages", g.gateway))
        .header("x-api-key", "sk-gw-not-issued")
        .header("content-type", "application/json")
        .body(json!({"model": "support-chat", "max_tokens": 8, "messages": [{"role": "user", "content": "x"}]}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
    let v: Value = serde_json::from_str(&res.text().await.unwrap()).unwrap();
    assert_eq!(v["error"]["type"], "authentication_error");
    assert!(g.seen.lock().unwrap().is_empty());

    let live = client()
        .get(format!("{}/health/live", g.gateway))
        .send()
        .await
        .unwrap();
    assert_eq!(live.status(), 200);
    g.health.set_ready();
    let ready = client()
        .get(format!("{}/health/ready", g.gateway))
        .send()
        .await
        .unwrap();
    assert_eq!(ready.status(), 200);
}

#[test]
fn a_wrong_configuration_is_refused_with_its_reason() {
    assert!(
        Config::parse("listen = \"127.0.0.1:1\"\n[budget]\nperiod_secs = 1\nprovidrs = []")
            .unwrap_err()
            .contains("unknown field")
    );
    let base = |extra: &str| {
        format!(
            "listen = \"127.0.0.1:1\"\n[budget]\nperiod_secs = 60\n[[providers]]\nname = \"p\"\nkind = \"anthropic\"\nresidency = \"abroad\"\n{extra}"
        )
    };
    let try_build = |toml: String| -> Result<(), String> {
        let config = Config::parse(&toml)?;
        build(
            &config,
            Arc::new(ReqwestTransport::with_defaults().unwrap()),
            Arc::new(StaticCredentials::new()),
            Arc::new(RecordingAuditSink::new()),
            Arc::new(SystemClock),
            seiva_http::Health::starting(),
        )
        .map(|_| ())
    };
    let route = "[[routes]]\nname = \"r\"\nresidency = \"in_country\"\npublic = true\nupstreams = [\"p/m\"]\n";
    let key = format!(
        "[[keys]]\nsubject = \"k\"\nsha256 = \"{}\"\n",
        hash_key("k")
    );
    let err = try_build(base(&format!("routes = []\n{key}"))).err();
    assert!(err.is_some());
    // An in-country route over an abroad provider.
    let err = try_build(base(&format!("{route}{key}"))).unwrap_err();
    assert!(err.contains("abroad"), "{err}");
    // No keys at all.
    let ok_route = route.replace("in_country", "abroad");
    let err = try_build(base(&ok_route)).unwrap_err();
    assert!(err.contains("keys"), "{err}");
    // A malformed digest.
    let err = try_build(base(&format!(
        "{ok_route}[[keys]]\nsubject = \"k\"\nsha256 = \"abc\"\n"
    )))
    .unwrap_err();
    assert!(err.contains("sha256"), "{err}");
    assert!(try_build(base(&format!("{ok_route}{key}"))).is_ok());
}
