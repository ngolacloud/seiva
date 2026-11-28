//! Live proofs of the pgvector index against a real Postgres with pgvector.
//! Gated on `SEIVA_PGVECTOR_TEST_DSN` (a sqlx Postgres URL, e.g.
//! `postgres://postgres:postgres@localhost:5432/seiva_rag`); every test skips with
//! a message when it is unset, so the default `cargo test` stays offline.
//!
//! Each test owns its table, so they run in parallel on one database.

use std::sync::Arc;

use seiva_identity::Principal;
use seiva_rag::{
    Access, AccessFilter, Chunk, Document, InMemoryIndex, Ingestor, Provenance, RagError,
    SearchQuery, VectorIndex,
};
use seiva_rag_pgvector::sqlx::postgres::PgPoolOptions;
use seiva_rag_pgvector::sqlx::{self, PgPool};
use seiva_rag_pgvector::{AnnIndex, PgVectorConfig, PgVectorIndex};
use seiva_testing::llm::HashEmbedder;

const DSN: &str = "SEIVA_PGVECTOR_TEST_DSN";
const MODEL: &str = "hash-8";

async fn pool() -> Option<PgPool> {
    let Ok(dsn) = std::env::var(DSN) else {
        eprintln!("skipping: set {DSN} to a Postgres with pgvector to run the live proofs");
        return None;
    };
    Some(
        PgPoolOptions::new()
            .max_connections(8)
            .connect(&dsn)
            .await
            .expect("connect to the pgvector test database"),
    )
}

/// A fresh, migrated index in its own table.
async fn index(pool: &PgPool, table: &str, ann: AnnIndex) -> PgVectorIndex {
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP TABLE IF EXISTS {table}")))
        .execute(pool)
        .await
        .unwrap();
    let config = PgVectorConfig::new(8)
        .unwrap()
        .with_table(table)
        .unwrap()
        .with_ann(ann)
        .unwrap();
    let index = PgVectorIndex::new(pool.clone(), config);
    index.migrate().await.unwrap();
    index
}

fn access(tenant: Option<&str>, requires: Option<&str>) -> Access {
    Access {
        tenant: tenant.map(str::to_owned),
        requires: requires.map(str::to_owned),
    }
}

fn chunk(document: &str, ordinal: usize, vector: Vec<f32>, access: Access) -> Chunk {
    Chunk {
        id: format!("{document}#{ordinal}"),
        document: document.into(),
        ordinal,
        title: Some(format!("Título de {document}")),
        text: format!("texto {ordinal} de {document}"),
        access,
        source: Provenance {
            uri: format!("https://intranet/{document}"),
            writer: "hr-portal".into(),
        },
        model: MODEL.into(),
        vector,
    }
}

/// A unit vector `angle` radians from the x axis in the first plane.
fn at(angle: f32) -> Vec<f32> {
    let mut v = vec![0.0; 8];
    v[0] = angle.cos();
    v[1] = angle.sin();
    v
}

fn ids(hits: &[seiva_rag::Hit]) -> Vec<String> {
    hits.iter().map(|h| h.chunk.id.clone()).collect()
}

async fn search(
    index: &dyn VectorIndex,
    who: &Principal,
    vector: &[f32],
    k: usize,
) -> Vec<seiva_rag::Hit> {
    let filter = AccessFilter::of(who);
    index
        .search(&SearchQuery {
            vector,
            model: MODEL,
            filter: &filter,
            k,
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn llm08_tenant_and_permission_are_filtered_in_the_query_and_k_counts_visible_chunks() {
    let Some(pool) = pool().await else { return };
    let index = index(&pool, "rag_live_filter", AnnIndex::Hnsw).await;
    // Zeta's chunks and Acme's board chunks sit right on the query; Ana's sit
    // further away.
    for i in 0..20 {
        index
            .replace_document(
                &format!("zeta-{i}"),
                vec![chunk(
                    &format!("zeta-{i}"),
                    0,
                    at(0.001 * i as f32),
                    access(Some("zeta"), None),
                )],
            )
            .await
            .unwrap();
        index
            .replace_document(
                &format!("board-{i}"),
                vec![chunk(
                    &format!("board-{i}"),
                    0,
                    at(0.002 * i as f32),
                    access(Some("acme"), Some("docs.board")),
                )],
            )
            .await
            .unwrap();
    }
    index
        .replace_document(
            "acme-hr",
            vec![chunk(
                "acme-hr",
                0,
                at(0.6),
                access(Some("acme"), Some("docs.hr")),
            )],
        )
        .await
        .unwrap();
    index
        .replace_document(
            "acme-all",
            vec![chunk("acme-all", 0, at(0.9), access(Some("acme"), None))],
        )
        .await
        .unwrap();
    index
        .replace_document(
            "shared",
            vec![chunk("shared", 0, at(1.2), access(None, None))],
        )
        .await
        .unwrap();

    let ana = Principal::in_tenant("ana", "acme", ["docs.hr"]);
    let hits = search(&index, &ana, &at(0.0), 3).await;
    // The top 3 are Ana's three visible chunks, nearest first — none of the 40
    // nearer chunks she may not read pushed them out.
    assert_eq!(ids(&hits), vec!["acme-hr#0", "acme-all#0", "shared#0"]);
    assert!((hits[0].score - 0.6f32.cos()).abs() < 1e-5);
    // A principal with no tenant sees only shared chunks.
    let svc = Principal::new("svc", ["docs.hr", "docs.board"]);
    assert_eq!(
        ids(&search(&index, &svc, &at(0.0), 10).await),
        vec!["shared#0"]
    );
    // The board member of Acme sees the board's chunks, never Zeta's.
    let board = Principal::in_tenant("rui", "acme", ["docs.board"]);
    let hits = search(&index, &board, &at(0.0), 50).await;
    assert_eq!(hits.len(), 22);
    assert!(hits
        .iter()
        .all(|h| h.chunk.access.tenant.as_deref() != Some("zeta")));
    assert!(hits
        .iter()
        .all(|h| h.chunk.access.requires.as_deref() != Some("docs.hr")));
}

#[tokio::test]
async fn a_selective_filter_still_fills_k_through_the_hnsw_index() {
    let Some(pool) = pool().await else { return };
    // A pool on which the planner must use an index, so the HNSW path (with its
    // iterative scan) is the one under test, not a sequential scan. A small
    // `ef_search` makes one candidate batch too small to hold a visible chunk.
    let dsn = std::env::var(DSN).unwrap();
    let forced = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("SET enable_seqscan = off")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("SET hnsw.ef_search = 10")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&dsn)
        .await
        .unwrap();
    let setup = index(&pool, "rag_live_iterative", AnnIndex::Hnsw).await;
    // With a selective filter Postgres rightly prefers the `(model, tenant)` index
    // (an exact answer); without it, the only index left is HNSW.
    sqlx::query("DROP INDEX rag_live_iterative_access")
        .execute(&pool)
        .await
        .unwrap();
    // 1 500 invisible chunks around the query, 4 visible ones further out. With
    // `ef_search = 10` a single candidate batch holds no visible chunk at all.
    let mut values = Vec::new();
    for i in 0..1_500 {
        values.push(format!(
            "('z{i}', 'z{i}', 0, NULL, 't', 'zeta', NULL, 'u', 'w', '{MODEL}', \
             '[{}, {}, 0, 0, 0, 0, 0, 0]')",
            (0.0005 * i as f32).cos(),
            (0.0005 * i as f32).sin()
        ));
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO rag_live_iterative (id, document, ordinal, title, body, tenant, requires, \
         source_uri, source_writer, model, embedding) VALUES {}",
        values.join(",")
    )))
    .execute(&pool)
    .await
    .unwrap();
    for (i, angle) in [1.0f32, 1.1, 1.2, 1.3].iter().enumerate() {
        let d = format!("acme-{i}");
        setup
            .replace_document(
                &d,
                vec![chunk(&d, 0, at(*angle), access(Some("acme"), None))],
            )
            .await
            .unwrap();
    }
    sqlx::query("ANALYZE rag_live_iterative")
        .execute(&pool)
        .await
        .unwrap();
    let index = PgVectorIndex::new(forced.clone(), setup.config().clone());
    let ana = Principal::in_tenant("ana", "acme", Vec::<String>::new());
    let hits = search(&index, &ana, &at(0.0), 3).await;
    assert_eq!(ids(&hits), vec!["acme-0#0", "acme-1#0", "acme-2#0"]);

    // The plan really is the HNSW index.
    let mut tx = forced.begin().await.unwrap();
    sqlx::query("SET LOCAL hnsw.iterative_scan = strict_order")
        .execute(&mut *tx)
        .await
        .unwrap();
    let plan: Vec<String> = sqlx::query_scalar(
        "EXPLAIN SELECT id FROM rag_live_iterative WHERE model = 'hash-8' \
         AND (tenant IS NULL OR tenant = 'acme') ORDER BY embedding <=> '[1,0,0,0,0,0,0,0]' LIMIT 3",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    assert!(
        plan.iter()
            .any(|l| l.contains("rag_live_iterative_embedding_hnsw")),
        "{plan:#?}"
    );
}

#[tokio::test]
async fn scores_and_order_match_the_in_memory_index() {
    let Some(pool) = pool().await else { return };
    let pg = Arc::new(index(&pool, "rag_live_parity", AnnIndex::Exact).await);
    let mem = Arc::new(InMemoryIndex::new());
    let embedder = Arc::new(HashEmbedder::new(8));
    let docs = [
        (
            "leave",
            "Pedir férias com trinta dias de antecedência ao gestor.",
            access(Some("acme"), None),
        ),
        (
            "refunds",
            "O estorno de um pagamento é aprovado pela tesouraria.",
            access(Some("acme"), Some("docs.finance")),
        ),
        (
            "security",
            "Nunca partilhar a palavra-passe nem o cartão de acesso.",
            access(None, None),
        ),
        (
            "zeta",
            "Pedir férias na Zeta exige o formulário azul.",
            access(Some("zeta"), None),
        ),
    ];
    for index in [pg.clone() as Arc<dyn VectorIndex>, mem.clone()] {
        let ingestor = Ingestor::new(embedder.clone(), MODEL, index);
        for (id, text, access) in &docs {
            ingestor
                .ingest(&Document {
                    id: (*id).into(),
                    title: None,
                    text: (*text).into(),
                    access: access.clone(),
                    source: Provenance {
                        uri: format!("https://intranet/{id}"),
                        writer: "hr-portal".into(),
                    },
                })
                .await
                .unwrap();
        }
    }
    let ana = Principal::in_tenant("ana", "acme", ["docs.finance"]);
    let q = seiva_llm::Embedder::embed(&*embedder, MODEL, &["pedir férias ao gestor".into()])
        .await
        .unwrap()
        .vectors
        .remove(0);
    let from_pg = search(&*pg, &ana, &q, 10).await;
    let from_mem = search(&*mem, &ana, &q, 10).await;
    assert_eq!(ids(&from_pg), ids(&from_mem));
    assert_eq!(from_pg.len(), 3, "zeta's chunk is not Ana's");
    for (a, b) in from_pg.iter().zip(&from_mem) {
        assert!(
            (a.score - b.score).abs() < 1e-6,
            "{} vs {}",
            a.score,
            b.score
        );
        assert_eq!(
            a.chunk, b.chunk,
            "the chunk comes back whole, provenance included"
        );
    }
}

#[tokio::test]
async fn replacing_a_document_is_one_transaction() {
    let Some(pool) = pool().await else { return };
    let index = index(&pool, "rag_live_replace", AnnIndex::Hnsw).await;
    let a = access(Some("acme"), None);
    index
        .replace_document(
            "policy",
            (0..3)
                .map(|i| chunk("policy", i, at(i as f32), a.clone()))
                .collect(),
        )
        .await
        .unwrap();
    assert_eq!(index.len().await.unwrap(), 3);
    // A shorter version replaces the whole document.
    index
        .replace_document("policy", vec![chunk("policy", 0, at(0.5), a.clone())])
        .await
        .unwrap();
    assert_eq!(index.len().await.unwrap(), 1);
    // A version that fails in the database (two chunks with one id) leaves the
    // previous version whole.
    let mut twice = chunk("policy", 1, at(0.1), a.clone());
    twice.id = "policy#0".into();
    let err = index
        .replace_document(
            "policy",
            vec![chunk("policy", 0, at(0.2), a.clone()), twice],
        )
        .await
        .unwrap_err();
    assert!(matches!(err, RagError::Index(_)));
    let ana = Principal::in_tenant("ana", "acme", Vec::<String>::new());
    let hits = search(&index, &ana, &at(0.5), 10).await;
    assert_eq!(ids(&hits), vec!["policy#0"]);
    assert!((hits[0].score - 1.0).abs() < 1e-5, "still the 0.5 version");
    // An empty list deletes it.
    index.replace_document("policy", Vec::new()).await.unwrap();
    assert!(index.is_empty().await.unwrap());
}

#[tokio::test]
async fn concurrent_writers_of_one_document_never_interleave() {
    let Some(pool) = pool().await else { return };
    let index = Arc::new(index(&pool, "rag_live_concurrent", AnnIndex::Hnsw).await);
    let mut tasks = Vec::new();
    for version in 1..=8usize {
        let index = index.clone();
        tasks.push(tokio::spawn(async move {
            let chunks = (0..version)
                .map(|i| {
                    let mut c = chunk("doc", i, at(0.1 * i as f32 + 0.01), access(None, None));
                    c.text = format!("v{version}");
                    c
                })
                .collect();
            index.replace_document("doc", chunks).await
        }));
    }
    for t in tasks {
        t.await.unwrap().unwrap();
    }
    // Exactly one version survives, whole.
    let anyone = Principal::new("svc", Vec::<String>::new());
    let hits = search(&*index, &anyone, &at(0.0), 100).await;
    let version = hits[0].chunk.text.clone();
    assert!(hits.iter().all(|h| h.chunk.text == version));
    assert_eq!(hits.len().to_string(), version.trim_start_matches('v'));
}

#[tokio::test]
async fn bad_chunks_and_queries_are_refused_before_the_database() {
    let Some(pool) = pool().await else { return };
    let index = index(&pool, "rag_live_refusals", AnnIndex::Hnsw).await;
    let a = access(None, None);
    let mut no_writer = chunk("d", 0, at(0.0), a.clone());
    no_writer.source.writer = " ".into();
    assert!(matches!(
        index.replace_document("d", vec![no_writer]).await,
        Err(RagError::NoProvenance(_))
    ));
    let short = chunk("d", 0, vec![1.0; 4], a.clone());
    assert!(index.replace_document("d", vec![short]).await.is_err());
    let zero = chunk("d", 0, vec![0.0; 8], a.clone());
    assert!(index.replace_document("d", vec![zero]).await.is_err());
    let mut nan = chunk("d", 0, at(0.0), a.clone());
    nan.vector[3] = f32::NAN;
    assert!(index.replace_document("d", vec![nan]).await.is_err());
    let elsewhere = chunk("other", 0, at(0.0), a.clone());
    assert!(matches!(
        index.replace_document("d", vec![elsewhere]).await,
        Err(RagError::Invalid(_))
    ));
    assert!(index.is_empty().await.unwrap());
    // A query of the wrong dimension is an error, not an empty answer.
    let filter = AccessFilter::of(&Principal::new("svc", Vec::<String>::new()));
    let err = index
        .search(&SearchQuery {
            vector: &[1.0, 0.0],
            model: MODEL,
            filter: &filter,
            k: 3,
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("dimensions"), "{err}");
}

#[tokio::test]
async fn only_chunks_of_the_querys_model_compare() {
    let Some(pool) = pool().await else { return };
    let index = index(&pool, "rag_live_models", AnnIndex::Hnsw).await;
    let mut other = chunk("other-model", 0, at(0.0), access(None, None));
    other.model = "another-embedder".into();
    index
        .replace_document("other-model", vec![other])
        .await
        .unwrap();
    index
        .replace_document("mine", vec![chunk("mine", 0, at(1.0), access(None, None))])
        .await
        .unwrap();
    let anyone = Principal::new("svc", Vec::<String>::new());
    assert_eq!(
        ids(&search(&index, &anyone, &at(0.0), 5).await),
        vec!["mine#0"]
    );
}

#[tokio::test]
async fn migrate_is_idempotent_and_refuses_another_dimension() {
    let Some(pool) = pool().await else { return };
    let index = index(&pool, "rag_live_migrate", AnnIndex::Hnsw).await;
    index.migrate().await.unwrap();
    // Two replicas migrating at once.
    let (a, b) = tokio::join!(index.migrate(), index.migrate());
    a.unwrap();
    b.unwrap();
    let wider = PgVectorIndex::new(
        pool.clone(),
        PgVectorConfig::new(16)
            .unwrap()
            .with_table("rag_live_migrate")
            .unwrap(),
    );
    let err = wider.migrate().await.unwrap_err();
    assert!(err.to_string().contains("vector(8)"), "{err}");
    // IVFFlat builds too.
    let ivf = index_ivf(&pool).await;
    ivf.replace_document("d", vec![chunk("d", 0, at(0.0), access(None, None))])
        .await
        .unwrap();
    let anyone = Principal::new("svc", Vec::<String>::new());
    assert_eq!(ids(&search(&ivf, &anyone, &at(0.1), 1).await), vec!["d#0"]);
}

async fn index_ivf(pool: &PgPool) -> PgVectorIndex {
    index(pool, "rag_live_ivfflat", AnnIndex::IvfFlat { lists: 4 }).await
}
