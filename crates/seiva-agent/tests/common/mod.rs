//! A facade with a read and a write tool, a gateway over a scripted model, and a
//! transport that serves HTTP requests from an axum router in-process.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use seiva::{facade, tools, WireSchema};
use seiva_ai_gateway::{BudgetScope, Caller, Gateway, InMemoryBudget};
use seiva_contract::Problem;
use seiva_http::Ctx;
use seiva_identity::Principal;
use seiva_llm::{
    HttpRequest, HttpResponse, HttpTransport, LlmError, Method, Residency, Route, Router,
};
use seiva_testing::llm::ScriptedModel;
use seiva_testing::{FixedClock, RecordingAuditSink};
use serde::{Deserialize, Serialize};

/// Command DTO: refund part of a payment.
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct RefundPayment {
    pub payment: String,
    pub amount_cents: u64,
}

/// A payment, as shown.
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct PaymentView {
    pub id: String,
    pub seen_by: String,
}

#[derive(Debug)]
pub enum PayError {
    NotFound,
}

impl From<PayError> for Problem {
    fn from(_: PayError) -> Self {
        Problem::not_found("no such payment")
    }
}

#[facade(version = "v1")]
pub trait Payments {
    /// Show a payment by its id.
    #[requires("payments.read")]
    #[tool(effect = "read")]
    async fn show(&self, ctx: &Ctx, id: String) -> Result<PaymentView, PayError>;

    /// Refund a captured payment, fully or partly.
    #[requires("payments.refund")]
    #[tool(effect = "write")]
    async fn refund(&self, ctx: &Ctx, cmd: RefundPayment) -> Result<PaymentView, PayError>;
}

#[derive(Clone, Default)]
pub struct PaymentsService {
    pub refunds: Arc<Mutex<Vec<(String, String, u64)>>>,
}

#[tools]
impl Payments for PaymentsService {
    async fn show(&self, ctx: &Ctx, id: String) -> Result<PaymentView, PayError> {
        Ok(PaymentView {
            id,
            seen_by: ctx.principal().subject.clone(),
        })
    }

    async fn refund(&self, ctx: &Ctx, cmd: RefundPayment) -> Result<PaymentView, PayError> {
        let by = ctx.principal().subject.clone();
        self.refunds
            .lock()
            .unwrap()
            .push((by.clone(), cmd.payment.clone(), cmd.amount_cents));
        Ok(PaymentView {
            id: cmd.payment,
            seen_by: by,
        })
    }
}

/// A gateway over `model` on route `agent` (abroad, masking on), letting a caller
/// with `agent.tools` offer any tool.
pub fn gateway(model: &ScriptedModel) -> (Gateway, RecordingAuditSink) {
    let router = Router::builder()
        .provider(Arc::new(model.clone()), Residency::Abroad)
        .route(
            Route::new("agent", Residency::Abroad)
                .requires("llm.agent")
                .upstream(model_name(model), "m"),
        )
        .retry(seiva_llm::RetryPolicy {
            max_attempts: 1,
            backoff_base_ms: 0,
        })
        .build()
        .unwrap();
    let audit = RecordingAuditSink::new();
    let gw = Gateway::builder(router)
        .budget(Arc::new(
            InMemoryBudget::new(
                Arc::new(FixedClock::at_unix_secs(0)),
                Duration::from_secs(86_400),
            )
            .per(BudgetScope::Principal, 1_000_000),
        ))
        .audit(Arc::new(audit.clone()))
        .any_tool("agent.tools")
        .build()
        .unwrap();
    (gw, audit)
}

fn model_name(model: &ScriptedModel) -> String {
    use seiva_llm::ChatProvider;
    model.name().to_owned()
}

pub fn ana() -> Caller {
    Caller::new(Principal::in_tenant(
        "ana",
        "acme",
        [
            "llm.agent",
            "agent.tools",
            "payments.read",
            "payments.refund",
        ],
    ))
}

pub fn reader() -> Caller {
    Caller::new(Principal::in_tenant(
        "rui",
        "acme",
        ["llm.agent", "agent.tools", "payments.read"],
    ))
}

/// Serves `HttpRequest`s from an axum router, in-process.
#[derive(Clone)]
pub struct RouterTransport(pub axum::Router);

#[async_trait]
impl HttpTransport for RouterTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, LlmError> {
        use tower::ServiceExt;
        let path = request
            .url
            .split_once("://")
            .map_or(request.url.as_str(), |(_, r)| r);
        let path = &path[path.find('/').unwrap_or(path.len())..];
        let mut builder = axum::http::Request::builder()
            .method(match request.method {
                Method::Get => "GET",
                _ => "POST",
            })
            .uri(path);
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let req = builder.body(axum::body::Body::from(request.body)).unwrap();
        let res = self
            .0
            .clone()
            .oneshot(req)
            .await
            .map_err(|e| LlmError::Transport(e.to_string()))?;
        let status = res.status().as_u16();
        let headers: Vec<(String, String)> = res
            .headers()
            .iter()
            .filter_map(|(n, v)| {
                v.to_str()
                    .ok()
                    .map(|v| (n.as_str().to_owned(), v.to_owned()))
            })
            .collect();
        let mut body = Vec::new();
        let mut stream = res.into_body().into_data_stream();
        while let Some(chunk) = stream.next().await {
            body.extend_from_slice(&chunk.map_err(|e| LlmError::Transport(e.to_string()))?);
        }
        let mut out = HttpResponse::from_bytes(status, body);
        for (n, v) in headers {
            out = out.with_header(n, v);
        }
        Ok(out)
    }
}
