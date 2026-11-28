# Retrieval (RAG)

> **Status: phases G6 and G7 of [ADR 0048](../adr/0048-ai-gateway-and-llm-harness.md)**,
> in `seiva-rag` and `seiva-rag-pgvector`.

Retrieval-augmented generation answers from the organisation's own documents. Two things
make it safe to put in front of people: a search that only ever finds what the reader may
read, and an answer that shows where it comes from.

## Index with provenance

```rust
let ingestor = Ingestor::new(embedder.clone(), "embed-model", index.clone());
ingestor.ingest(&Document {
    id: "hr-leave-policy".into(),
    title: Some("Leave policy".into()),
    text,
    access: Access { tenant: Some("acme".into()), requires: Some("docs.hr".into()) },
    source: Provenance { uri: "https://intranet/hr/leave".into(), writer: "hr-portal".into() },
}).await?;
```

A document without a source and a writer is refused, and every chunk keeps both — a
poisoned chunk can be traced and removed (OWASP LLM04). Ingesting the same id again
replaces the document; `remove` deletes it.

The embedding model sees the text. Embed through the [gateway](language-models.md#embeddings-through-the-gateway)
— `gateway.embedder(caller)` in-process, or `seiva-llm`'s OpenAI-compatible embedder
pointed at the gateway server — and the texts are masked, budgeted and audited like any
model call. Masking changes what is embedded (`‹TAX_ID_1›`, not the number), so a search
for the exact value of a masked identifier will not find it; where that matters, embed on
an `in_country` route with in-country masking off.

## Search as the reader

```rust
let retriever = Retriever::new(embedder, "embed-model", index);
let hits = retriever.search(&caller, "How early must I ask for leave?", 4).await?;
```

The index port has a single search, and it takes an `AccessFilter` built from the
caller's principal: chunks of their tenant (or shared ones, `tenant: None`) whose
`requires` they hold. An adapter applies it **inside the query** — a pgvector `WHERE`
before the `ORDER BY … LIMIT k` — so the top `k` are counted among visible chunks and an
invisible chunk never leaves the store (OWASP LLM08). A principal with no tenant sees only
shared documents.

## On Postgres: pgvector

```rust
use seiva_rag_pgvector::{sqlx::PgPool, AnnIndex, PgVectorConfig, PgVectorIndex};

let pool = PgPool::connect(&database_url).await?;
let index = PgVectorIndex::new(pool, PgVectorConfig::new(1024)?);   // HNSW, cosine
index.migrate().await?;   // CREATE EXTENSION IF NOT EXISTS vector, the table, the indexes
let index: Arc<dyn VectorIndex> = Arc::new(index);
```

`seiva-rag-pgvector` keeps the chunks in one table (provenance columns `NOT NULL`, the
column's dimension checked on every write and query) and puts the reader's filter in the
SQL `WHERE`, before `ORDER BY embedding <=> $query LIMIT k`. With an approximate index it
turns on pgvector's iterative index scans (pgvector 0.8 or later), so a selective filter —
one tenant among many — still fills `k` instead of returning fewer rows;
`AnnIndex::Exact` skips the approximate index for exact results, `AnnIndex::IvfFlat` builds
IVFFlat instead of HNSW. Scores are the same cosine as `InMemoryIndex`'s, and
`replace_document` is one transaction, so a reader sees one version of a document or the
other. `migrate` refuses a table of another dimension rather than altering it; the DDL is
also available (`PgVectorConfig::migrations`) for a deployment that runs its own
migrations. The crate links sqlx, so it stays out of Seiva's default build.

## Answers that cite

```rust
let a = answer(&gateway, "docs", &caller, &retriever, "How early must I ask for leave?", 4, 512).await?;
for c in &a.citations { println!("[{}] {}", c.n, c.uri); }
if a.uncited { /* show the answer as unsupported */ }
```

The chunks are numbered and wrapped as untrusted data, the model is told to cite them as
`[n]` and to say when they do not answer, and the call goes through the
[gateway](language-models.md) (masking, budget, audit). The reply's markers are checked:
`uncited` marks an answer with no source, `unknown_markers` lists invented references
(OWASP LLM09). When nothing the caller may read matches, no model is called.

## For agents

`SearchTool` is a `search_documents` tool for [`seiva-agent`](agents.md): it searches as
the person the agent acts for and returns numbered excerpts with their sources.

## Related

- [Agents](agents.md) · [Language models](language-models.md) · [Security](security.md)

---

*Part of the [Seiva documentation](../index.md).*
