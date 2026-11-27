//! The server shape, request by request: the OpenAI and Anthropic endpoints over
//! a scripted model, whole and streamed, with gateway keys, errors in each
//! client's shape, and the routes a caller may see.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use seiva_ai_gateway::server::{hash_key, router, GatewayKeys, KeyEntry, ServerState};
use seiva_ai_gateway::{BudgetScope, Gateway, InMemoryBudget};
use seiva_identity::Principal;
use seiva_llm::{ChatResponse, Content, Residency, Route, Router, StopReason, Usage};
use seiva_testing::llm::ScriptedModel;
use seiva_testing::{FixedClock, RecordingAuditSink};
use serde_json::{json, Value};
use tower::ServiceExt;

const ANA_KEY: &str = "sk-gw-ana-1111111111111111";
const RITA_KEY: &str = "sk-gw-rita-2222222222222222";
const NIF: &str = "5417123456";

fn app(model: ScriptedModel) -> (axum::Router, ScriptedModel, RecordingAuditSink) {
    let local = ScriptedModel::new("local").reply_text("local");
    let llm = Router::builder()
        .provider(Arc::new(model.clone()), Residency::Abroad)
        .provider(Arc::new(local), Residency::InCountry)
        .route(
            Route::new("support-chat", Residency::Abroad)
                .requires("llm.support")
                .upstream("cloud", "m"),
        )
        .route(
            Route::new("hr-assistant", Residency::InCountry)
                .requires("llm.hr")
                .upstream("local", "llama"),
        )
        .route(
            Route::new("finance", Residency::Abroad)
                .requires("llm.finance")
                .upstream("cloud", "m"),
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
        .tool("customer_lookup", "customers.read")
        .area_in_country("hr")
        .build()
        .unwrap();
    let keys = GatewayKeys::new()
        .with(KeyEntry {
            sha256: hash_key(ANA_KEY),
            principal: Principal::in_tenant("ana", "acme", ["llm.support", "customers.read"]),
            area: Some("support".into()),
        })
        .unwrap()
        .with(KeyEntry {
            sha256: hash_key(RITA_KEY),
            principal: Principal::new("rita", ["llm.support", "llm.hr"]),
            area: Some("hr".into()),
        })
        .unwrap();
    (router(ServerState::new(gateway, keys)), model, audit)
}

async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    auth: Option<(&str, &str)>,
    body: Option<Value>,
) -> (StatusCode, String, Option<String>) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some((name, value)) = auth {
        req = req.header(name, value);
    }
    let req = req
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let rid = res
        .headers()
        .get("request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap(), rid)
}

fn bearer(key: &str) -> Option<(&'static str, String)> {
    Some(("authorization", format!("Bearer {key}")))
}

fn sent(model: &ScriptedModel) -> String {
    serde_json::to_string(&model.calls()[0].request).unwrap()
}

#[tokio::test]
async fn openai_clients_get_an_openai_answer_and_the_model_never_sees_the_nif() {
    let (app, model, audit) =
        app(ScriptedModel::new("cloud").reply_text("O NIF ‹TAX_ID_1› está activo."));
    let auth = bearer(ANA_KEY).unwrap();
    let (status, body, rid) = call(
        &app,
        "POST",
        "/v1/chat/completions",
        Some((auth.0, &auth.1)),
        Some(json!({"model": "support-chat", "messages": [
            {"role": "system", "content": "Sê breve."},
            {"role": "user", "content": format!("O NIF {NIF} está activo?")}
        ]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["object"], "chat.completion");
    assert_eq!(v["model"], "support-chat");
    assert_eq!(
        v["choices"][0]["message"]["content"],
        format!("O NIF {NIF} está activo.")
    );
    assert_eq!(v["choices"][0]["finish_reason"], "stop");
    assert!(v["usage"]["prompt_tokens"].as_u64().unwrap() > 0);
    assert!(rid.unwrap().starts_with("req_"));
    let s = sent(&model);
    assert!(s.contains("‹TAX_ID_1›") && !s.contains(NIF), "{s}");
    assert_eq!(audit.entries()[0].actor, "ana");
    assert_eq!(audit.entries()[0].tenant.as_deref(), Some("acme"));
}

#[tokio::test]
async fn anthropic_clients_get_an_anthropic_answer_with_tool_calls() {
    let model = ScriptedModel::new("cloud").reply(ChatResponse::new(
        "m",
        vec![
            Content::text("Vou procurar."),
            Content::ToolUse {
                id: "toolu_1".into(),
                name: "customer_lookup".into(),
                input: json!({"nif": "‹TAX_ID_1›"}),
            },
        ],
        StopReason::ToolUse,
        Usage {
            input_tokens: 20,
            output_tokens: 8,
            cache_read_tokens: 100,
            cache_write_tokens: 0,
        },
    ));
    let (app, model, _) = app(model);
    let (status, body, _) = call(
        &app,
        "POST",
        "/v1/messages?beta=true",
        Some(("x-api-key", ANA_KEY)),
        Some(json!({
            "model": "support-chat", "max_tokens": 512,
            "system": [{"type": "text", "text": "És o assistente de suporte.", "cache_control": {"type": "ephemeral"}}],
            "messages": [{"role": "user", "content": [{"type": "text", "text": format!("Procura o NIF {NIF}")}]}],
            "tools": [{"name": "customer_lookup", "description": "Find a customer", "input_schema": {"type": "object"}}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["type"], "message");
    assert_eq!(v["stop_reason"], "tool_use");
    assert_eq!(
        v["content"][1],
        json!({"type": "tool_use", "id": "toolu_1", "name": "customer_lookup", "input": {"nif": NIF}})
    );
    assert_eq!(v["usage"]["cache_read_input_tokens"], 100);
    assert!(!sent(&model).contains(NIF));
}

#[tokio::test]
async fn tool_results_from_an_openai_client_reach_the_model_labelled_and_masked() {
    let (app, model, _) = app(ScriptedModel::new("cloud").reply_text("Feito."));
    let auth = bearer(ANA_KEY).unwrap();
    let (status, body, _) = call(
        &app,
        "POST",
        "/v1/chat/completions",
        Some((auth.0, &auth.1)),
        Some(json!({"model": "support-chat", "max_tokens": 100,
            "tools": [{"type": "function", "function": {"name": "customer_lookup", "parameters": {"type": "object"}}}],
            "messages": [
                {"role": "user", "content": "Procura o cliente C-7"},
                {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "customer_lookup", "arguments": "{\"id\":\"C-7\"}"}}]},
                {"role": "tool", "tool_call_id": "call_1", "content": format!("{{\"nif\": \"NIF {NIF}\", \"email\": \"c7@exemplo.ao\"}}")}
            ]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let req = &model.calls()[0].request;
    let Content::ToolResult {
        content,
        tool_use_id,
        ..
    } = &req.messages[2].content[0]
    else {
        panic!("{req:?}")
    };
    assert_eq!(tool_use_id, "call_1");
    assert!(content.starts_with("<untrusted_data"));
    assert!(
        !content.contains(NIF) && !content.contains("c7@exemplo.ao"),
        "{content}"
    );
}

#[tokio::test]
async fn keys_errors_and_limits_answer_in_each_clients_shape() {
    let (app, model, _) = app(ScriptedModel::new("cloud"));
    let msg = |route: &str, max: u64| json!({"model": route, "max_tokens": max, "messages": [{"role": "user", "content": "olá"}]});

    let (s, b, _) = call(
        &app,
        "POST",
        "/v1/messages",
        Some(("x-api-key", "sk-wrong")),
        Some(msg("support-chat", 10)),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(
        serde_json::from_str::<Value>(&b).unwrap()["error"]["type"],
        "authentication_error"
    );

    let (s, b, _) = call(
        &app,
        "POST",
        "/v1/chat/completions",
        None,
        Some(msg("support-chat", 10)),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(
        serde_json::from_str::<Value>(&b).unwrap()["error"]["type"],
        "authentication_error"
    );

    let (s, b, _) = call(
        &app,
        "POST",
        "/v1/messages",
        Some(("x-api-key", ANA_KEY)),
        Some(msg("finance", 10)),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert_eq!(
        serde_json::from_str::<Value>(&b).unwrap()["error"]["type"],
        "permission_error"
    );

    let (s, _, _) = call(
        &app,
        "POST",
        "/v1/messages",
        Some(("x-api-key", ANA_KEY)),
        Some(msg("nope", 10)),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Rita's area keeps data in the country: the abroad route is refused.
    let (s, b, _) = call(
        &app,
        "POST",
        "/v1/messages",
        Some(("x-api-key", RITA_KEY)),
        Some(msg("support-chat", 10)),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");

    let (s, b, _) = call(
        &app,
        "POST",
        "/v1/messages",
        Some(("x-api-key", ANA_KEY)),
        Some(msg("support-chat", 100_000)),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(b.contains("cap"), "{b}");

    let img = json!({"model": "support-chat", "messages": [{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://evil.example/x.png"}}]}]});
    let auth = bearer(ANA_KEY).unwrap();
    let (s, b, _) = call(
        &app,
        "POST",
        "/v1/chat/completions",
        Some((auth.0, &auth.1)),
        Some(img),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(b.contains("does not fetch"), "{b}");

    assert!(
        model.calls().is_empty(),
        "no refused request reached the model"
    );
}

#[tokio::test]
async fn a_spent_budget_is_a_429() {
    let (app, _, _) = app(ScriptedModel::new("cloud"));
    let (s, b, _) = call(
        &app,
        "POST",
        "/v1/messages",
        Some(("x-api-key", ANA_KEY)),
        Some(json!({"model": "support-chat", "max_tokens": 60_000, "messages": [{"role": "user", "content": "olá"}]})),
    )
    .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{b}");
    assert_eq!(
        serde_json::from_str::<Value>(&b).unwrap()["error"]["type"],
        "rate_limit_error"
    );
}

#[tokio::test]
async fn models_lists_only_the_routes_the_caller_may_use() {
    let (app, _, _) = app(ScriptedModel::new("cloud"));
    let ids = |b: &str| -> Vec<String> {
        serde_json::from_str::<Value>(b).unwrap()["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let auth = bearer(ANA_KEY).unwrap();
    let (s, b, _) = call(&app, "GET", "/v1/models", Some((auth.0, &auth.1)), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(ids(&b), vec!["support-chat"]);
    // Rita may use both, but her area keeps data in the country.
    let (_, b, _) = call(
        &app,
        "GET",
        "/v1/models",
        Some(("x-api-key", RITA_KEY)),
        None,
    )
    .await;
    assert_eq!(ids(&b), vec!["hr-assistant"]);
    let v: Value = serde_json::from_str(&b).unwrap();
    assert_eq!(v["data"][0]["type"], "model");
    assert_eq!(v["data"][0]["object"], "model");
}

#[tokio::test]
async fn count_tokens_estimates_without_calling_a_model() {
    let (app, model, _) = app(ScriptedModel::new("cloud"));
    let (s, b, _) = call(
        &app,
        "POST",
        "/v1/messages/count_tokens",
        Some(("x-api-key", ANA_KEY)),
        Some(json!({"model": "support-chat", "messages": [{"role": "user", "content": "uma frase com algumas palavras"}]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(
        serde_json::from_str::<Value>(&b).unwrap()["input_tokens"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(model.calls().is_empty());
}

fn data_lines(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn an_openai_stream_ends_with_done_and_carries_unmasked_tool_calls() {
    let model = ScriptedModel::new("cloud").reply(ChatResponse::new(
        "m",
        vec![
            Content::text("A procurar ‹TAX_ID_1›."),
            Content::ToolUse {
                id: "call_9".into(),
                name: "customer_lookup".into(),
                input: json!({"nif": "‹TAX_ID_1›"}),
            },
        ],
        StopReason::ToolUse,
        Usage {
            input_tokens: 5,
            output_tokens: 3,
            ..Usage::default()
        },
    ));
    let (app, _, audit) = app(model);
    let auth = bearer(ANA_KEY).unwrap();
    let (s, body, _) = call(
        &app,
        "POST",
        "/v1/chat/completions",
        Some((auth.0, &auth.1)),
        Some(json!({"model": "support-chat", "stream": true, "stream_options": {"include_usage": true},
            "tools": [{"type": "function", "function": {"name": "customer_lookup", "parameters": {"type": "object"}}}],
            "messages": [{"role": "user", "content": format!("NIF {NIF}")}]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let lines = data_lines(&body);
    assert_eq!(lines.last().map(String::as_str), Some("[DONE]"));
    let chunks: Vec<Value> = lines
        .iter()
        .filter(|l| *l != "[DONE]")
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let text: String = chunks
        .iter()
        .filter_map(|c| c["choices"][0]["delta"]["content"].as_str())
        .collect();
    assert_eq!(text, format!("A procurar {NIF}."));
    let call = chunks
        .iter()
        .find_map(|c| c["choices"][0]["delta"]["tool_calls"].get(0).cloned())
        .unwrap();
    assert_eq!(
        call["function"]["arguments"],
        format!("{{\"nif\":\"{NIF}\"}}")
    );
    assert!(chunks
        .iter()
        .any(|c| c["choices"][0]["finish_reason"] == "tool_calls"));
    assert!(chunks.iter().any(|c| c["usage"]["completion_tokens"] == 3));
    assert_eq!(audit.entries()[0].outcome, "ok");
}

#[tokio::test]
async fn an_anthropic_stream_follows_the_event_sequence() {
    let (app, _, _) = app(ScriptedModel::new("cloud").reply_text("Olá, ‹TAX_ID_1›!"));
    let (s, body, _) = call(
        &app,
        "POST",
        "/v1/messages",
        Some(("x-api-key", ANA_KEY)),
        Some(
            json!({"model": "support-chat", "max_tokens": 64, "stream": true,
            "messages": [{"role": "user", "content": format!("Sou o NIF {NIF}")}]}),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let events: Vec<&str> = body
        .lines()
        .filter_map(|l| l.strip_prefix("event: "))
        .collect();
    assert_eq!(
        events,
        vec![
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop"
        ]
    );
    let deltas: String = data_lines(&body)
        .iter()
        .filter_map(|d| serde_json::from_str::<Value>(d).ok())
        .filter_map(|v| v["delta"]["text"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(deltas, format!("Olá, {NIF}!"));
}
