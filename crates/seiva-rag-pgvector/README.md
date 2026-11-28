# seiva-rag-pgvector

The Postgres + pgvector `VectorIndex` of [`seiva-rag`](../seiva-rag) — phase G7 of
[ADR 0048](https://github.com/ngolacloud/seiva/blob/main/docs/adr/0048-ai-gateway-and-llm-harness.md).
DORM-free; built on sqlx (the query layer of `seiva-data-sqlx`), so it is kept out of
Seiva's default build.

```rust
use seiva_rag_pgvector::{sqlx::PgPool, AnnIndex, PgVectorConfig, PgVectorIndex};

let pool = PgPool::connect(&database_url).await?;
let config = PgVectorConfig::new(1024)?            // the embedding model's dimension
    .with_table("hr_chunks")?                      // default: seiva_rag_chunks
    .with_ann(AnnIndex::Hnsw)?;                    // or IvfFlat { lists }, or Exact
let index = PgVectorIndex::new(pool, config);
index.migrate().await?;                            // extension, table, indexes — idempotent

let ingestor = Ingestor::new(gateway.embedder(indexer), "docs-embed", Arc::new(index.clone()));
let retriever = Retriever::new(gateway.embedder(reader), "docs-embed", Arc::new(index));
```

| Guarantee | How |
|---|---|
| The reader's filter is in the query (OWASP LLM08) | `WHERE model = $m AND (tenant IS NULL OR tenant = $t) AND (requires IS NULL OR requires = ANY($perms))` before `ORDER BY embedding <=> $q LIMIT k`: `k` counts visible chunks, an invisible chunk never leaves the database |
| A selective filter still fills `k` | with HNSW or IVFFlat the search runs with pgvector's iterative index scans (`hnsw.iterative_scan = strict_order`, `ivfflat.iterative_scan = relaxed_order`; pgvector ≥ 0.8, checked by `migrate`) |
| The same scores as `InMemoryIndex` | rows ordered by cosine distance; each score is `seiva_rag::cosine` of the stored vector |
| One version of a document at a time | `replace_document` deletes and inserts in one transaction, under an advisory lock on the document |
| Provenance kept (OWASP LLM04) | `source_uri`, `source_writer` are `NOT NULL` and non-empty, checked before the write and by the schema |
| The dimension holds | every chunk and query vector must have the column's dimension, finite and not all zero; `migrate` refuses an existing table of another dimension |

The DDL is also returned by `PgVectorConfig::migrations()` for deployments that run their
own migration tool. `CREATE EXTENSION vector` needs a role allowed to create it: where
the application's role is not, a DBA runs that statement once.

## Testing

The live tests run against a real Postgres with pgvector and skip without one:

```console
$ podman run -d -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=seiva_rag -p 5432:5432 pgvector/pgvector:pg16
$ SEIVA_PGVECTOR_TEST_DSN=postgres://postgres:postgres@localhost:5432/seiva_rag \
    cargo test --manifest-path crates/seiva-rag-pgvector/Cargo.toml
```

## License

MIT OR Apache-2.0.
