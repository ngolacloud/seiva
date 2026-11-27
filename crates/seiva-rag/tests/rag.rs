//! Retrieval with permissions end to end: the OWASP rows of this phase (LLM04,
//! LLM08, LLM09), the answer through the gateway, and the agent tool.

use std::sync::Arc;
use std::time::Duration;

use seiva_agent::{Agent, Cancel};
use seiva_ai_gateway::{BudgetScope, Caller, Gateway, InMemoryBudget};
use seiva_identity::Principal;
use seiva_llm::{Residency, Route, Router};
use seiva_rag::{
    answer, Access, Document, InMemoryIndex, IngestReport, Ingestor, Provenance, RagError,
    Retriever, SearchTool,
};
use seiva_testing::llm::{HashEmbedder, ScriptedModel};
use seiva_testing::{FixedClock, RecordingAuditSink};
use serde_json::json;

const MODEL: &str = "hash-64";
const NIF: &str = "5417123456";

fn doc(id: &str, tenant: Option<&str>, requires: Option<&str>, text: &str) -> Document {
    Document {
        id: id.into(),
        title: Some(format!("Documento {id}")),
        text: text.into(),
        access: Access {
            tenant: tenant.map(str::to_owned),
            requires: requires.map(str::to_owned),
        },
        source: Provenance {
            uri: format!("https://docs.example/{id}"),
            writer: "rh-portal".into(),
        },
    }
}

struct Corpus {
    index: Arc<InMemoryIndex>,
    ingestor: Ingestor,
    retriever: Retriever,
}

async fn corpus() -> Corpus {
    let index = Arc::new(InMemoryIndex::new());
    let embedder = Arc::new(HashEmbedder::new(64));
    let ingestor = Ingestor::new(embedder.clone(), MODEL, index.clone());
    let docs = [
        // Zeta's document is the closest to the question — and must never reach Ana.
        doc(
            "zeta-policy",
            Some("zeta"),
            None,
            "política de férias prazo de férias trinta dias pedido de férias zeta",
        ),
        doc(
            "acme-policy",
            Some("acme"),
            None,
            "política de férias da acme o pedido de férias faz-se com trinta dias",
        ),
        doc(
            "acme-salaries",
            Some("acme"),
            Some("docs.payroll"),
            "tabela de salários e pedido de férias pagas",
        ),
        doc(
            "shared-law",
            None,
            None,
            "a lei geral do trabalho fixa as férias anuais",
        ),
        doc(
            "acme-onboarding",
            Some("acme"),
            None,
            &format!("boas-vindas pedido de férias no primeiro ano NIF {NIF}"),
        ),
    ];
    for d in &docs {
        ingestor.ingest(d).await.unwrap();
    }
    let retriever = Retriever::new(embedder, MODEL, index.clone());
    Corpus {
        index,
        ingestor,
        retriever,
    }
}

fn ana() -> Caller {
    Caller::new(Principal::in_tenant(
        "ana",
        "acme",
        ["llm.docs", "agent.tools"],
    ))
}

fn ids(hits: &[seiva_rag::Hit]) -> Vec<String> {
    hits.iter().map(|h| h.chunk.document.clone()).collect()
}

/// LLM08 — tenant A never retrieves tenant B's chunk, a chunk behind a permission
/// stays hidden from a reader without it, and `k` counts visible chunks only.
#[tokio::test]
async fn llm08_a_reader_finds_only_what_they_may_read() {
    let c = corpus().await;
    let hits = c
        .retriever
        .search(&ana(), "pedido de férias prazo trinta dias", 10)
        .await
        .unwrap();
    let found = ids(&hits);
    assert!(!found.contains(&"zeta-policy".to_owned()), "{found:?}");
    assert!(
        !found.contains(&"acme-salaries".to_owned()),
        "behind docs.payroll: {found:?}"
    );
    assert!(found.contains(&"acme-policy".to_owned()) && found.contains(&"shared-law".to_owned()));

    // k = 2 still yields two hits: the filter runs before the top-k, not after.
    assert_eq!(
        c.retriever
            .search(&ana(), "pedido de férias prazo trinta dias", 2)
            .await
            .unwrap()
            .len(),
        2
    );

    // With the permission, the payroll document appears.
    let payroll = Caller::new(Principal::in_tenant("rui", "acme", ["docs.payroll"]));
    assert!(ids(&c
        .retriever
        .search(&payroll, "salários férias pagas", 10)
        .await
        .unwrap())
    .contains(&"acme-salaries".to_owned()));

    // A principal with no tenant sees only shared documents.
    let service = Caller::new(Principal::new("batch", Vec::<String>::new()));
    assert_eq!(
        ids(&c.retriever.search(&service, "férias", 10).await.unwrap()),
        vec!["shared-law".to_owned()]
    );
}

/// LLM04 — a document without provenance is not indexed; every chunk keeps its
/// source and writer; re-ingesting replaces, removing deletes.
#[tokio::test]
async fn llm04_provenance_is_required_and_kept() {
    let c = corpus().await;
    let mut anonymous = doc("leak", Some("acme"), None, "texto sem origem");
    anonymous.source.writer = String::new();
    assert_eq!(
        c.ingestor.ingest(&anonymous).await,
        Err(RagError::NoProvenance("leak".into()))
    );

    let hits = c
        .retriever
        .search(&ana(), "política de férias da acme", 1)
        .await
        .unwrap();
    assert_eq!(
        hits[0].chunk.source,
        Provenance {
            uri: "https://docs.example/acme-policy".into(),
            writer: "rh-portal".into()
        }
    );

    let before = c.index.len();
    let report = c
        .ingestor
        .ingest(&doc(
            "acme-policy",
            Some("acme"),
            None,
            "nova versão da política",
        ))
        .await
        .unwrap();
    assert_eq!(
        report,
        IngestReport {
            document: "acme-policy".into(),
            chunks: 1
        }
    );
    assert_eq!(c.index.len(), before, "replaced, not duplicated");
    c.ingestor.remove("acme-policy").await.unwrap();
    assert!(
        !ids(&c.retriever.search(&ana(), "política", 10).await.unwrap())
            .contains(&"acme-policy".to_owned())
    );
}

fn gateway(model: &ScriptedModel) -> Gateway {
    let router = Router::builder()
        .provider(Arc::new(model.clone()), Residency::Abroad)
        .route(
            Route::new("docs", Residency::Abroad)
                .requires("llm.docs")
                .upstream("cloud", "m"),
        )
        .build()
        .unwrap();
    Gateway::builder(router)
        .budget(Arc::new(
            InMemoryBudget::new(
                Arc::new(FixedClock::at_unix_secs(0)),
                Duration::from_secs(60),
            )
            .per(BudgetScope::Principal, 100_000),
        ))
        .audit(Arc::new(RecordingAuditSink::new()))
        .any_tool("agent.tools")
        .build()
        .unwrap()
}

/// LLM09 — the answer's `[n]` markers are checked against the documents it was
/// given; documents reach the model labelled and masked.
#[tokio::test]
async fn llm09_answers_cite_and_the_unsupported_ones_say_so() {
    let c = corpus().await;
    let model = ScriptedModel::new("cloud")
        .reply_text("O pedido de férias faz-se com 30 dias de antecedência [1] [9].")
        .reply_text("Penso que são 30 dias.");
    let gw = gateway(&model);

    let a = answer(
        &gw,
        "docs",
        &ana(),
        &c.retriever,
        "Com que antecedência peço férias no primeiro ano? boas-vindas acme",
        4,
        256,
    )
    .await
    .unwrap();
    assert_eq!(a.citations.len(), 1);
    assert_eq!(a.citations[0].uri, a.hits[0].chunk.source.uri);
    assert_eq!(
        a.unknown_markers,
        vec![9],
        "an invented reference is flagged"
    );
    assert!(!a.uncited);

    // The model got the documents labelled as untrusted, with the NIF masked.
    let sent = serde_json::to_string(&model.calls()[0].request).unwrap();
    assert!(
        sent.contains("untrusted_data") && sent.contains("cite"),
        "{sent}"
    );
    assert!(
        a.hits.iter().any(|h| h.chunk.document == "acme-onboarding"),
        "the document with the NIF was retrieved"
    );
    assert!(!sent.contains(NIF) && sent.contains("‹TAX_ID_1›"), "{sent}");

    let b = answer(
        &gw,
        "docs",
        &ana(),
        &c.retriever,
        "Com que antecedência peço férias? acme",
        3,
        256,
    )
    .await
    .unwrap();
    assert!(b.uncited, "no [n]: unsupported");
}

#[tokio::test]
async fn nothing_readable_found_means_no_model_call() {
    let index = Arc::new(InMemoryIndex::new());
    let embedder = Arc::new(HashEmbedder::new(64));
    Ingestor::new(embedder.clone(), MODEL, index.clone())
        .ingest(&doc("zeta-only", Some("zeta"), None, "férias da zeta"))
        .await
        .unwrap();
    let model = ScriptedModel::new("cloud");
    let a = answer(
        &gateway(&model),
        "docs",
        &ana(),
        &Retriever::new(embedder, MODEL, index),
        "férias",
        3,
        64,
    )
    .await
    .unwrap();
    assert!(a.uncited && a.text.is_empty() && a.hits.is_empty());
    assert!(model.calls().is_empty());
}

#[tokio::test]
async fn chunks_of_another_embedding_model_are_never_compared() {
    let index = Arc::new(InMemoryIndex::new());
    let embedder = Arc::new(HashEmbedder::new(64));
    Ingestor::new(embedder.clone(), "old-model", index.clone())
        .ingest(&doc("old", Some("acme"), None, "férias"))
        .await
        .unwrap();
    let hits = Retriever::new(embedder, MODEL, index)
        .search(&ana(), "férias", 5)
        .await
        .unwrap();
    assert!(hits.is_empty());
}

#[tokio::test]
async fn the_agent_tool_searches_as_the_person() {
    let c = corpus().await;
    let model = ScriptedModel::new("cloud")
        .reply_tool_call(
            "t1",
            "search_documents",
            json!({"query": "pedido de férias prazo trinta dias"}),
        )
        .reply_text("São 30 dias [1].");
    let agent = Agent::builder(gateway(&model), "docs")
        .tool(Arc::new(SearchTool::new(c.retriever.clone(), 4)))
        .build()
        .unwrap();
    let run = agent
        .run(
            &ana(),
            None,
            "Com que antecedência peço férias?",
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert_eq!(run.answer, "São 30 dias [1].");
    let second = serde_json::to_string(&model.calls()[1].request).unwrap();
    assert!(second.contains("docs.example/acme-policy"), "{second}");
    assert!(
        !second.contains("zeta"),
        "another tenant's document never reaches the model: {second}"
    );
}
