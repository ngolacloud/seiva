//! Embeddings through the gateway (ADR 0048, G7): the same stages as a chat call,
//! in the same order, each failing closed — over a recording embedder.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use seiva_ai_gateway::{
    AuditMode, BudgetKey, BudgetScope, Caller, Gateway, GatewayBuilder, GatewayError, GuardContext,
    Guardrail, InMemoryBudget, SizeLimit, Stage, Verdict,
};
use seiva_identity::Principal;
use seiva_llm::{
    ChatProvider, ChatRequest, Embedder, Embeddings, LlmError, Residency, Route, RouteKind, Router,
    Usage,
};
use seiva_ports::{AuditEntry, AuditSink, PortResult};
use seiva_testing::llm::{HashEmbedder, ScriptedModel};
use seiva_testing::{FixedClock, RecordingAuditSink};

const NIF: &str = "5417123456";

/// Records every batch it is asked to embed; answers with [`HashEmbedder`]'s
/// vectors and `usage_per_text` input tokens per text.
#[derive(Clone, Default)]
struct Recording {
    seen: Arc<Mutex<Vec<Vec<String>>>>,
    usage_per_text: u64,
    broken: bool,
}

impl Recording {
    fn seen(&self) -> Vec<Vec<String>> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl Embedder for Recording {
    async fn embed(&self, model: &str, inputs: &[String]) -> Result<Embeddings, LlmError> {
        self.seen.lock().unwrap().push(inputs.to_vec());
        let mut e = HashEmbedder::new(8).embed(model, inputs).await?;
        if self.broken {
            e.vectors[0] = vec![f32::NAN; 8];
        }
        e.usage = Usage {
            input_tokens: self.usage_per_text * inputs.len() as u64,
            ..Usage::default()
        };
        Ok(e)
    }
}

struct Fixture {
    cloud: Recording,
    local: Recording,
    chat: ScriptedModel,
    audit: RecordingAuditSink,
    budget: Arc<InMemoryBudget>,
}

fn fixture_with(cloud: Recording) -> (Fixture, GatewayBuilder) {
    let local = Recording::default();
    let chat = ScriptedModel::new("cloud").reply_text("ok");
    let llm = Router::builder()
        .provider(
            Arc::new(chat.clone()) as Arc<dyn ChatProvider>,
            Residency::Abroad,
        )
        .embedder("cloud", Arc::new(cloud.clone()), Residency::Abroad)
        .embedder("local", Arc::new(local.clone()), Residency::InCountry)
        .route(
            Route::new("chat", Residency::Abroad)
                .requires("llm.chat")
                .upstream("cloud", "m"),
        )
        .route(
            Route::new("docs-embed", Residency::Abroad)
                .with_kind(RouteKind::Embedding)
                .requires("llm.embed")
                .upstream("cloud", "text-embedding"),
        )
        .route(
            Route::new("hr-embed", Residency::InCountry)
                .with_kind(RouteKind::Embedding)
                .requires("llm.embed")
                .upstream("local", "bge-m3"),
        )
        .retry(seiva_llm::RetryPolicy {
            max_attempts: 1,
            backoff_base_ms: 0,
        })
        .build()
        .unwrap();
    let audit = RecordingAuditSink::new();
    let budget = Arc::new(
        InMemoryBudget::new(
            Arc::new(FixedClock::at_unix_secs(0)),
            Duration::from_secs(86_400),
        )
        .per(BudgetScope::Principal, 1_000),
    );
    let builder = Gateway::builder(llm)
        .budget(budget.clone())
        .audit(Arc::new(audit.clone()))
        .area_in_country("hr");
    (
        Fixture {
            cloud,
            local,
            chat,
            audit,
            budget,
        },
        builder,
    )
}

fn fixture() -> (Fixture, GatewayBuilder) {
    fixture_with(Recording {
        usage_per_text: 7,
        ..Recording::default()
    })
}

fn indexer() -> Caller {
    Caller::new(Principal::in_tenant(
        "indexer",
        "acme",
        ["llm.embed", "llm.chat"],
    ))
    .with_request_id("req-e")
}

fn used(f: &Fixture, who: &str) -> u64 {
    f.budget.used(&BudgetKey::new(BudgetScope::Principal, who))
}

#[tokio::test]
async fn an_embedding_call_runs_the_stages_in_order_and_is_audited_without_content() {
    let (f, b) = fixture();
    let gw = b.build().unwrap();
    let texts = vec![
        format!("O cliente com NIF {NIF} pediu um estorno."),
        format!("Confirmar o NIF {NIF} antes de pagar."),
    ];
    let gated = gw.embed(&indexer(), "docs-embed", texts).await.unwrap();
    assert_eq!(
        gated.report.stages,
        vec![
            Stage::Identify,
            Stage::Authorize,
            Stage::BudgetReserve,
            Stage::Mask,
            Stage::GuardInput,
            Stage::Model,
            Stage::BudgetSettle,
            Stage::Audit,
        ]
    );
    assert_eq!(gated.embeddings.vectors.len(), 2);
    assert_eq!(
        gated.embeddings.upstream.as_deref(),
        Some("cloud/text-embedding")
    );
    // LLM02: the embedder never saw the NIF; the same value got one placeholder.
    let seen = f.cloud.seen();
    assert_eq!(seen[0][0], "O cliente com NIF ‹TAX_ID_1› pediu um estorno.");
    assert_eq!(seen[0][1], "Confirmar o NIF ‹TAX_ID_1› antes de pagar.");
    assert_eq!(gated.report.masked.get("TAX_ID"), Some(&1));
    // The provider's usage is what is spent.
    assert_eq!(used(&f, "indexer"), 14);

    let entries = f.audit.entries();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(
        (e.operation.as_str(), e.outcome.as_str()),
        ("llm.embed", "ok")
    );
    assert_eq!(e.object.as_deref(), Some("docs-embed"));
    let detail = |k: &str| {
        e.details
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(detail("inputs").as_deref(), Some("2"));
    assert_eq!(detail("dimensions").as_deref(), Some("8"));
    assert_eq!(detail("usage_in").as_deref(), Some("14"));
    assert_eq!(detail("masked").as_deref(), Some("TAX_ID=1"));
    assert_eq!(detail("upstream").as_deref(), Some("cloud/text-embedding"));
    let all = format!("{:?}", e.details);
    assert!(!all.contains(NIF) && !all.contains("estorno"), "{all}");
}

#[tokio::test]
async fn a_chat_route_is_not_an_embedding_route_and_the_other_way_round() {
    let (f, b) = fixture();
    let gw = b.build().unwrap();
    let err = gw
        .embed(&indexer(), "chat", vec!["texto".into()])
        .await
        .unwrap_err();
    assert_eq!(
        err,
        GatewayError::WrongRouteKind {
            route: "chat".into(),
            wanted: RouteKind::Embedding
        }
    );
    assert_eq!(err.http_status(), 400);
    let err = gw
        .complete(&indexer(), "docs-embed", ChatRequest::new(10).user("olá"))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        GatewayError::WrongRouteKind {
            wanted: RouteKind::Chat,
            ..
        }
    ));
    // Refused before the budget and the model.
    assert!(f.cloud.seen().is_empty() && f.chat.calls().is_empty());
    assert_eq!(used(&f, "indexer"), 0);
    // Both refusals were audited.
    let outcomes: Vec<String> = f
        .audit
        .entries()
        .iter()
        .map(|e| e.outcome.clone())
        .collect();
    assert_eq!(outcomes, vec!["wrong_route_kind", "wrong_route_kind"]);
}

#[tokio::test]
async fn a_route_declared_for_both_serves_both() {
    let embedder = Recording::default();
    let chat = ScriptedModel::new("p").reply_text("ok");
    let llm = Router::builder()
        .provider(Arc::new(chat), Residency::InCountry)
        .embedder("p", Arc::new(embedder), Residency::InCountry)
        .route(
            Route::new("both", Residency::InCountry)
                .with_kind(RouteKind::ChatAndEmbedding)
                .public()
                .upstream("p", "m"),
        )
        .build()
        .unwrap();
    let gw = Gateway::builder(llm)
        .budget(Arc::new(InMemoryBudget::new(
            Arc::new(FixedClock::at_unix_secs(0)),
            Duration::from_secs(60),
        )))
        .audit(Arc::new(RecordingAuditSink::new()))
        .build()
        .unwrap();
    let caller = Caller::new(Principal::new("ana", Vec::<String>::new()));
    assert!(gw.embed(&caller, "both", vec!["a".into()]).await.is_ok());
    assert!(gw
        .complete(&caller, "both", ChatRequest::new(10).user("a"))
        .await
        .is_ok());
}

#[tokio::test]
async fn identity_permission_and_residency_are_checked_first() {
    let (f, b) = fixture();
    let gw = b.build().unwrap();
    let anonymous = Caller::new(Principal::new("", ["llm.embed"]));
    assert_eq!(
        gw.embed(&anonymous, "docs-embed", vec!["a".into()])
            .await
            .unwrap_err(),
        GatewayError::Unauthenticated
    );
    let outsider = Caller::new(Principal::new("rui", ["llm.chat"]));
    assert!(matches!(
        gw.embed(&outsider, "docs-embed", vec!["a".into()]).await,
        Err(GatewayError::Forbidden { .. })
    ));
    // A caller without the permission is told it lacks it, not the route's kind.
    assert!(matches!(
        gw.embed(&outsider, "hr-embed", vec!["a".into()]).await,
        Err(GatewayError::Forbidden { .. })
    ));
    let hr = Caller::new(Principal::new("rita", ["llm.embed"])).in_area("hr");
    assert!(matches!(
        gw.embed(&hr, "docs-embed", vec!["a".into()]).await,
        Err(GatewayError::Residency { .. })
    ));
    assert!(gw.embed(&hr, "hr-embed", vec!["a".into()]).await.is_ok());
    assert!(matches!(
        gw.embed(&indexer(), "nope", vec!["a".into()]).await,
        Err(GatewayError::Model(LlmError::UnknownRoute(_)))
    ));
    assert!(matches!(
        gw.embed(&indexer(), "docs-embed", vec![]).await,
        Err(GatewayError::Blocked { .. })
    ));
    assert!(f.cloud.seen().is_empty());
    assert_eq!(f.local.seen().len(), 1);
}

#[tokio::test]
async fn llm07_a_credential_refuses_the_embedding_call() {
    let (f, b) = fixture();
    let gw = b.build().unwrap();
    let err = gw
        .embed(
            &indexer(),
            "hr-embed",
            vec!["ligar com postgres://app:s3cr3t@db.acme.ao/app".into()],
        )
        .await
        .unwrap_err();
    assert!(matches!(err, GatewayError::Blocked { ref stage, .. } if stage == "mask"));
    assert!(f.local.seen().is_empty());
    assert_eq!(used(&f, "indexer"), 0, "the hold is given back");
}

#[tokio::test]
async fn masking_follows_the_chat_rule_on_in_country_routes() {
    // Masked by default…
    let (f, b) = fixture();
    let gw = b.build().unwrap();
    gw.embed(&indexer(), "hr-embed", vec![format!("NIF {NIF}")])
        .await
        .unwrap();
    assert_eq!(f.local.seen()[0][0], "NIF ‹TAX_ID_1›");
    // …in clear when the gateway does not mask in the country…
    let (f, b) = fixture();
    let gw = b.mask_in_country(false).build().unwrap();
    gw.embed(&indexer(), "hr-embed", vec![format!("NIF {NIF}")])
        .await
        .unwrap();
    assert_eq!(f.local.seen()[0][0], format!("NIF {NIF}"));
    // …and always masked abroad.
    gw.embed(&indexer(), "docs-embed", vec![format!("NIF {NIF}")])
        .await
        .unwrap();
    assert_eq!(f.cloud.seen()[0][0], "NIF ‹TAX_ID_1›");
}

#[tokio::test]
async fn guardrails_see_the_masked_texts() {
    struct NoSalaries(Arc<Mutex<Vec<String>>>);
    #[async_trait]
    impl Guardrail for NoSalaries {
        fn name(&self) -> &str {
            "no_salaries"
        }
        async fn check_embedding_input(
            &self,
            _: &GuardContext<'_>,
            inputs: &[String],
        ) -> Result<Verdict, String> {
            self.0.lock().unwrap().extend(inputs.iter().cloned());
            Ok(if inputs.iter().any(|t| t.contains("salário")) {
                Verdict::Block("salaries are not indexed".into())
            } else {
                Verdict::Allow
            })
        }
    }
    let saw = Arc::new(Mutex::new(Vec::new()));
    let (f, b) = fixture();
    let gw = b
        .guard(Arc::new(NoSalaries(saw.clone())))
        // A chat guardrail covers embeddings through the default hook.
        .guard(Arc::new(SizeLimit { max_chars: 40 }))
        .build()
        .unwrap();
    let err = gw
        .embed(
            &indexer(),
            "docs-embed",
            vec![format!("NIF {NIF}"), "o salário de março".into()],
        )
        .await
        .unwrap_err();
    assert!(matches!(err, GatewayError::Blocked { ref stage, .. } if stage == "no_salaries"));
    assert_eq!(saw.lock().unwrap()[0], "NIF ‹TAX_ID_1›");
    let err = gw
        .embed(&indexer(), "docs-embed", vec!["palavra ".repeat(10)])
        .await
        .unwrap_err();
    assert!(matches!(err, GatewayError::Blocked { ref stage, .. } if stage == "size_limit"));
    assert!(f.cloud.seen().is_empty());
}

#[tokio::test]
async fn llm10_the_budget_bounds_embeddings_and_fails_closed() {
    let (f, b) = fixture();
    let gw = b.build().unwrap();
    // 1 000 tokens per principal; each text of 3 600 characters holds 901.
    let big = "a".repeat(3_600);
    gw.embed(&indexer(), "docs-embed", vec![big.clone()])
        .await
        .unwrap();
    // Settled at the provider's count (7), so there is room for one more hold…
    assert_eq!(used(&f, "indexer"), 7);
    gw.embed(&indexer(), "docs-embed", vec![big.clone()])
        .await
        .unwrap();
    // …until the holds no longer fit.
    let err = gw
        .embed(&indexer(), "docs-embed", vec![big.clone(), big])
        .await
        .unwrap_err();
    assert!(matches!(err, GatewayError::BudgetExceeded { .. }));
}

#[tokio::test]
async fn without_the_providers_usage_the_estimate_is_spent() {
    let (f, b) = fixture_with(Recording::default());
    let gw = b.build().unwrap();
    gw.embed(&indexer(), "docs-embed", vec!["a".repeat(40)])
        .await
        .unwrap();
    assert_eq!(used(&f, "indexer"), 11);
}

#[tokio::test]
async fn a_malformed_answer_is_refused_and_still_counted() {
    let (f, b) = fixture_with(Recording {
        usage_per_text: 5,
        broken: true,
        ..Recording::default()
    });
    let gw = b.build().unwrap();
    let err = gw
        .embed(&indexer(), "docs-embed", vec!["a".into()])
        .await
        .unwrap_err();
    assert!(matches!(err, GatewayError::Model(LlmError::Decode(_))));
    assert_eq!(used(&f, "indexer"), 5);
}

struct FailingSink;

#[async_trait]
impl AuditSink for FailingSink {
    async fn record(&self, _: &AuditEntry) -> PortResult<()> {
        Err("disk full".into())
    }
}

#[tokio::test]
async fn vectors_that_cannot_be_audited_are_withheld() {
    let (_, b) = fixture();
    let gw = b.audit(Arc::new(FailingSink)).build().unwrap();
    let err = gw
        .embed(&indexer(), "docs-embed", vec!["a".into()])
        .await
        .unwrap_err();
    assert!(matches!(err, GatewayError::AuditFailed(_)));
    let (_, b) = fixture();
    let gw = b
        .audit(Arc::new(FailingSink))
        .audit_mode(AuditMode::BestEffort)
        .build()
        .unwrap();
    assert!(gw
        .embed(&indexer(), "docs-embed", vec!["a".into()])
        .await
        .is_ok());
}

#[tokio::test]
async fn the_gateway_is_an_embedder_for_one_caller() {
    let (f, b) = fixture();
    let gw = b.build().unwrap();
    let embedder: Arc<dyn Embedder> = Arc::new(gw.embedder(indexer()));
    let e = embedder
        .embed("docs-embed", &[format!("NIF {NIF}")])
        .await
        .unwrap();
    assert_eq!(e.vectors.len(), 1);
    assert_eq!(f.cloud.seen()[0][0], "NIF ‹TAX_ID_1›");
    // A refusal reads like the server's answer.
    let outsider = Arc::new(gw.embedder(Caller::new(Principal::new("rui", ["llm.chat"]))));
    let err = outsider
        .embed("docs-embed", &["a".into()])
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Status { status: 403, .. }));
    let err = embedder.embed("chat", &["a".into()]).await.unwrap_err();
    assert!(matches!(err, LlmError::Status { status: 400, .. }));
}
