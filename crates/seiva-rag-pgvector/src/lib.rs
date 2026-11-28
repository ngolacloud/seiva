//! `seiva-rag-pgvector` — the Postgres + pgvector [`VectorIndex`] of `seiva-rag`
//! (ADR 0048, phase G7).
//!
//! ```text
//! search(query, AccessFilter::of(caller))
//!   SELECT … FROM chunks
//!    WHERE model = $model
//!      AND (tenant IS NULL OR tenant = $tenant)            -- the reader's tenant
//!      AND (requires IS NULL OR requires = ANY($perms))    -- the reader's permissions
//!    ORDER BY embedding <=> $query                         -- cosine distance
//!    LIMIT $k
//! ```
//!
//! - **The filter is in the query.** The reader's tenant and permissions are in
//!   the `WHERE`, before the nearest-neighbour `ORDER BY … LIMIT k`: the top `k`
//!   are counted among the chunks the reader may see, and an invisible chunk never
//!   leaves the database (OWASP LLM08). With an approximate index (HNSW, IVFFlat)
//!   the search turns on pgvector's *iterative index scans* for its transaction,
//!   so a selective filter keeps the scan going instead of returning fewer than
//!   `k` rows; [`AnnIndex::Exact`] gives exact results at the cost of a scan.
//! - **The same scores as [`InMemoryIndex`](seiva_rag::InMemoryIndex).** Rows are
//!   ordered by pgvector's cosine distance; each hit's score is
//!   [`seiva_rag::cosine`] of the stored vector, the function the in-memory index
//!   uses, so moving from one to the other changes no score.
//! - **Provenance kept, dimension checked.** `source_uri` and `source_writer` are
//!   `NOT NULL` and non-empty in the schema too; every vector must have the
//!   column's dimension, finite and not all zero (pgvector's cosine distance of a
//!   zero vector is undefined).
//! - **One transaction per document.** [`replace_document`](VectorIndex::replace_document)
//!   deletes the old chunks and inserts the new ones in one transaction, holding
//!   an advisory lock on the document, so a reader sees one version or the other
//!   and two writers of the same document never interleave.
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use seiva_rag_pgvector::{sqlx::PgPool, PgVectorConfig, PgVectorIndex};
//!
//! let pool = PgPool::connect("postgres://rag@db/rag").await?;
//! let index = PgVectorIndex::new(pool, PgVectorConfig::new(1024)?);
//! index.migrate().await?;   // CREATE EXTENSION IF NOT EXISTS vector, the table, HNSW
//! // `Arc::new(index)` is a `seiva_rag::VectorIndex` for the `Ingestor` and the `Retriever`.
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use async_trait::async_trait;
use seiva_rag::{cosine, Access, Chunk, Hit, Provenance, RagError, SearchQuery, VectorIndex};
use sqlx::postgres::PgRow;
use sqlx::{AssertSqlSafe, PgPool, Postgres, Row, Transaction};

/// The sqlx this crate is built on, so a composition root builds the pool with
/// the same version.
pub use sqlx;

/// The default table.
pub const DEFAULT_TABLE: &str = "seiva_rag_chunks";

/// pgvector's largest `vector` column.
pub const MAX_DIMENSIONS: u32 = 16_000;

/// pgvector's largest vector an HNSW or IVFFlat index can hold.
pub const MAX_INDEXED_DIMENSIONS: u32 = 2_000;

/// The oldest pgvector with iterative index scans (0.8.0), required for an
/// approximate index: without them a filtered search could return fewer than
/// `k` rows.
pub const MIN_PGVECTOR_FOR_ANN: (u32, u32) = (0, 8);

/// The nearest-neighbour index on the embedding column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum AnnIndex {
    /// HNSW with cosine distance (`vector_cosine_ops`) and pgvector's default
    /// build parameters. The default: good recall without training data.
    #[default]
    Hnsw,
    /// IVFFlat with cosine distance and `lists` lists. Build it once the table has
    /// data (pgvector chooses the centres from the rows present).
    IvfFlat {
        /// The number of lists (pgvector suggests rows ÷ 1000 up to 1M rows).
        lists: u32,
    },
    /// No approximate index: every search is exact (a scan of the reader's
    /// visible chunks). For small corpora, or where exact recall matters more than
    /// speed.
    Exact,
}

/// Where and how the chunks are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgVectorConfig {
    table: String,
    dimensions: u32,
    ann: AnnIndex,
}

impl PgVectorConfig {
    /// Vectors of `dimensions` components in [`DEFAULT_TABLE`], with an HNSW index.
    pub fn new(dimensions: u32) -> Result<Self, RagError> {
        if dimensions == 0 || dimensions > MAX_DIMENSIONS {
            return Err(RagError::Index(format!(
                "a pgvector column has 1 to {MAX_DIMENSIONS} dimensions, not {dimensions}"
            )));
        }
        Self {
            table: DEFAULT_TABLE.to_owned(),
            dimensions,
            ann: AnnIndex::Hnsw,
        }
        .checked()
    }

    /// Another table: lower-case letters, digits and `_`, starting with a letter or
    /// `_`, at most 40 characters (the index names add a suffix within Postgres's
    /// 63). Schema-qualify through the connection's `search_path`.
    pub fn with_table(mut self, table: impl Into<String>) -> Result<Self, RagError> {
        self.table = table.into();
        self.checked()
    }

    /// Another nearest-neighbour index.
    pub fn with_ann(mut self, ann: AnnIndex) -> Result<Self, RagError> {
        self.ann = ann;
        self.checked()
    }

    fn checked(self) -> Result<Self, RagError> {
        let t = self.table.as_bytes();
        let ok = !t.is_empty()
            && t.len() <= 40
            && (t[0].is_ascii_lowercase() || t[0] == b'_')
            && t.iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_');
        if !ok {
            return Err(RagError::Index(format!(
                "`{}` is not an accepted table name ([a-z_][a-z0-9_]*, at most 40 characters)",
                self.table
            )));
        }
        match self.ann {
            AnnIndex::Hnsw | AnnIndex::IvfFlat { .. }
                if self.dimensions > MAX_INDEXED_DIMENSIONS =>
            {
                Err(RagError::Index(format!(
                    "pgvector indexes vectors of at most {MAX_INDEXED_DIMENSIONS} dimensions; \
                     use AnnIndex::Exact for {}",
                    self.dimensions
                )))
            }
            AnnIndex::IvfFlat { lists: 0 } => Err(RagError::Index(
                "an IVFFlat index needs at least one list".into(),
            )),
            _ => Ok(self),
        }
    }

    /// The table.
    pub fn table(&self) -> &str {
        &self.table
    }

    /// The vectors' dimension.
    pub fn dimensions(&self) -> u32 {
        self.dimensions
    }

    /// The nearest-neighbour index.
    pub fn ann(&self) -> AnnIndex {
        self.ann
    }

    /// The DDL [`PgVectorIndex::migrate`] runs, in order — for a deployment that
    /// applies migrations with its own tool. Every statement is idempotent
    /// (`IF NOT EXISTS`).
    pub fn migrations(&self) -> Vec<String> {
        let t = &self.table;
        let d = self.dimensions;
        let mut out = vec![
            "CREATE EXTENSION IF NOT EXISTS vector".to_owned(),
            format!(
                "CREATE TABLE IF NOT EXISTS {t} (
    id            text PRIMARY KEY CHECK (id <> ''),
    document      text NOT NULL CHECK (document <> ''),
    ordinal       integer NOT NULL CHECK (ordinal >= 0),
    title         text,
    body          text NOT NULL,
    tenant        text,
    requires      text CHECK (requires IS NULL OR requires <> ''),
    source_uri    text NOT NULL CHECK (source_uri <> ''),
    source_writer text NOT NULL CHECK (source_writer <> ''),
    model         text NOT NULL CHECK (model <> ''),
    embedding     vector({d}) NOT NULL,
    indexed_at    timestamptz NOT NULL DEFAULT now(),
    UNIQUE (document, ordinal)
)"
            ),
            format!("CREATE INDEX IF NOT EXISTS {t}_access ON {t} (model, tenant)"),
        ];
        match self.ann {
            AnnIndex::Hnsw => out.push(format!(
                "CREATE INDEX IF NOT EXISTS {t}_embedding_hnsw ON {t} \
                 USING hnsw (embedding vector_cosine_ops)"
            )),
            AnnIndex::IvfFlat { lists } => out.push(format!(
                "CREATE INDEX IF NOT EXISTS {t}_embedding_ivfflat ON {t} \
                 USING ivfflat (embedding vector_cosine_ops) WITH (lists = {lists})"
            )),
            AnnIndex::Exact => {}
        }
        out
    }
}

/// A [`VectorIndex`] on Postgres with pgvector. Cheap to clone (the pool is).
#[derive(Clone)]
pub struct PgVectorIndex {
    pool: PgPool,
    config: PgVectorConfig,
}

impl PgVectorIndex {
    /// An index over `pool`. Run [`migrate`](Self::migrate) (or the DDL of
    /// [`PgVectorConfig::migrations`]) once before using it.
    pub fn new(pool: PgPool, config: PgVectorConfig) -> Self {
        Self { pool, config }
    }

    /// The configuration.
    pub fn config(&self) -> &PgVectorConfig {
        &self.config
    }

    /// Create the extension, the table and its indexes if they are missing, in
    /// one transaction under a database-wide advisory lock (two replicas starting
    /// together, or two indexes in one database, do not race), then check what is there: pgvector new enough for the configured
    /// index, and an existing table whose `embedding` column has the configured
    /// dimension — a mismatch is refused, never altered.
    ///
    /// `CREATE EXTENSION` needs a role allowed to create it; where the
    /// application's role is not, a DBA runs that one statement once and
    /// `migrate` finds the extension there.
    pub async fn migrate(&self) -> Result<(), RagError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        // One lock for the whole database, not per table: `CREATE EXTENSION` is
        // database-wide, and two migrations of different tables racing on it fail
        // with a duplicate key in `pg_extension`.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind("seiva-rag-pgvector:migrate")
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        for statement in self.config.migrations() {
            // The DDL is built from the validated configuration (a checked table
            // name and two integers), never from input.
            sqlx::query(AssertSqlSafe(statement))
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
        let version: String =
            sqlx::query_scalar("SELECT extversion FROM pg_extension WHERE extname = 'vector'")
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        if self.config.ann != AnnIndex::Exact && parse_version(&version) < MIN_PGVECTOR_FOR_ANN {
            return Err(RagError::Index(format!(
                "pgvector {version} has no iterative index scans, so a filtered search could \
                 return fewer than k chunks: upgrade to 0.8.0 or later, or use AnnIndex::Exact"
            )));
        }
        let column: Option<String> = sqlx::query_scalar(
            "SELECT format_type(atttypid, atttypmod) FROM pg_attribute \
             WHERE attrelid = to_regclass($1) AND attname = 'embedding' AND NOT attisdropped",
        )
        .bind(&self.config.table)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        let expected = format!("vector({})", self.config.dimensions);
        if column.as_deref() != Some(expected.as_str()) {
            return Err(RagError::Index(format!(
                "table `{}` keeps `{}`, not `{expected}`: an index of another dimension needs \
                 another table",
                self.config.table,
                column.as_deref().unwrap_or("no embedding column")
            )));
        }
        tx.commit().await.map_err(db)
    }

    /// How many chunks the table holds (every tenant's — an operator's count, not
    /// a reader's).
    pub async fn len(&self) -> Result<usize, RagError> {
        let sql = format!("SELECT count(*) FROM {}", self.config.table);
        let n: i64 = sqlx::query_scalar(AssertSqlSafe(sql))
            .fetch_one(&self.pool)
            .await
            .map_err(db)?;
        Ok(usize::try_from(n).unwrap_or(0))
    }

    /// Whether the table is empty.
    pub async fn is_empty(&self) -> Result<bool, RagError> {
        Ok(self.len().await? == 0)
    }

    /// A vector the column can hold: its dimension, finite, not all zero.
    fn check_vector(&self, v: &[f32], what: &str) -> Result<(), RagError> {
        let d = self.config.dimensions as usize;
        if v.len() != d {
            return Err(RagError::Index(format!(
                "{what} has {} dimensions; the index keeps {d}",
                v.len()
            )));
        }
        if v.iter().any(|x| !x.is_finite()) {
            return Err(RagError::Index(format!(
                "{what} has a component that is not finite"
            )));
        }
        if v.iter().all(|x| *x == 0.0) {
            return Err(RagError::Index(format!(
                "{what} is a zero vector, which has no cosine distance"
            )));
        }
        Ok(())
    }

    fn check_chunk(&self, document: &str, c: &Chunk) -> Result<(), RagError> {
        if c.document != document {
            return Err(RagError::Invalid(format!(
                "chunk `{}` belongs to `{}`, not `{document}`",
                c.id, c.document
            )));
        }
        if c.id.trim().is_empty() || c.model.trim().is_empty() {
            return Err(RagError::Invalid(format!(
                "a chunk of `{document}` has no id or no model"
            )));
        }
        if c.source.uri.trim().is_empty() || c.source.writer.trim().is_empty() {
            return Err(RagError::NoProvenance(document.to_owned()));
        }
        if c.access
            .requires
            .as_deref()
            .is_some_and(|p| p.trim().is_empty())
        {
            return Err(RagError::Invalid(format!(
                "chunk `{}` has an empty permission",
                c.id
            )));
        }
        if i32::try_from(c.ordinal).is_err() {
            return Err(RagError::Invalid(format!(
                "chunk `{}` has an ordinal out of range",
                c.id
            )));
        }
        self.check_vector(&c.vector, &format!("chunk `{}`", c.id))
    }

    async fn lock_document(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        document: &str,
    ) -> Result<(), RagError> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!(
                "seiva-rag-pgvector:{}:{document}",
                self.config.table
            ))
            .execute(&mut **tx)
            .await
            .map_err(db)?;
        Ok(())
    }
}

#[async_trait]
impl VectorIndex for PgVectorIndex {
    async fn replace_document(&self, document: &str, chunks: Vec<Chunk>) -> Result<(), RagError> {
        if document.trim().is_empty() {
            return Err(RagError::Invalid("a document needs an id".into()));
        }
        // Everything is checked before the transaction opens: a bad chunk never
        // leaves the old version half-deleted.
        for c in &chunks {
            self.check_chunk(document, c)?;
        }
        let t = &self.config.table;
        let delete = format!("DELETE FROM {t} WHERE document = $1");
        let insert = format!(
            "INSERT INTO {t} (id, document, ordinal, title, body, tenant, requires, \
             source_uri, source_writer, model, embedding) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::real[]::vector)"
        );
        let mut tx = self.pool.begin().await.map_err(db)?;
        self.lock_document(&mut tx, document).await?;
        sqlx::query(AssertSqlSafe(delete))
            .bind(document)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        for c in chunks {
            // The table name is validated; every value is bound.
            sqlx::query(AssertSqlSafe(insert.clone()))
                .bind(c.id)
                .bind(c.document)
                .bind(i32::try_from(c.ordinal).unwrap_or(i32::MAX))
                .bind(c.title)
                .bind(c.text)
                .bind(c.access.tenant)
                .bind(c.access.requires)
                .bind(c.source.uri)
                .bind(c.source.writer)
                .bind(c.model)
                .bind(c.vector)
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
        // Dropped without a commit on any error above: the old version stays.
        tx.commit().await.map_err(db)
    }

    async fn search(&self, query: &SearchQuery<'_>) -> Result<Vec<Hit>, RagError> {
        self.check_vector(query.vector, "the query")?;
        if query.k == 0 {
            return Ok(Vec::new());
        }
        let t = &self.config.table;
        let sql = format!(
            "SELECT id, document, ordinal, title, body, tenant, requires, source_uri, \
             source_writer, model, embedding::real[] AS embedding \
             FROM {t} \
             WHERE model = $2 \
               AND (tenant IS NULL OR tenant = $3) \
               AND (requires IS NULL OR requires = ANY($4)) \
             ORDER BY embedding <=> $1::real[]::vector \
             LIMIT $5"
        );
        let permissions: Vec<String> = query.filter.permissions().iter().cloned().collect();
        let k = i64::try_from(query.k).unwrap_or(i64::MAX);
        let mut tx = self.pool.begin().await.map_err(db)?;
        // Keep scanning the approximate index until `k` rows pass the filter
        // (pgvector ≥ 0.8), instead of filtering one fixed batch of candidates.
        let iterative = match self.config.ann {
            AnnIndex::Hnsw => Some("SET LOCAL hnsw.iterative_scan = strict_order"),
            AnnIndex::IvfFlat { .. } => Some("SET LOCAL ivfflat.iterative_scan = relaxed_order"),
            AnnIndex::Exact => None,
        };
        if let Some(set) = iterative {
            sqlx::query(set).execute(&mut *tx).await.map_err(db)?;
        }
        let rows = sqlx::query(AssertSqlSafe(sql))
            .bind(query.vector.to_vec())
            .bind(query.model)
            .bind(query.filter.tenant())
            .bind(permissions)
            .bind(k)
            .fetch_all(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        let mut hits = rows
            .iter()
            .map(|row| {
                let chunk = chunk_of(row)?;
                Ok(Hit {
                    score: cosine(query.vector, &chunk.vector),
                    chunk,
                })
            })
            .collect::<Result<Vec<Hit>, RagError>>()?;
        // The database ordered by distance; ties and an approximate index's
        // relaxed order are settled with the same score the in-memory index uses.
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(query.k);
        Ok(hits)
    }
}

fn chunk_of(row: &PgRow) -> Result<Chunk, RagError> {
    let ordinal: i32 = row.try_get("ordinal").map_err(db)?;
    Ok(Chunk {
        id: row.try_get("id").map_err(db)?,
        document: row.try_get("document").map_err(db)?,
        ordinal: usize::try_from(ordinal).unwrap_or(0),
        title: row.try_get("title").map_err(db)?,
        text: row.try_get("body").map_err(db)?,
        access: Access {
            tenant: row.try_get("tenant").map_err(db)?,
            requires: row.try_get("requires").map_err(db)?,
        },
        source: Provenance {
            uri: row.try_get("source_uri").map_err(db)?,
            writer: row.try_get("source_writer").map_err(db)?,
        },
        model: row.try_get("model").map_err(db)?,
        vector: row.try_get("embedding").map_err(db)?,
    })
}

fn db(e: sqlx::Error) -> RagError {
    RagError::Index(e.to_string())
}

/// `0.8.1` → `(0, 8)`; anything unreadable → `(0, 0)`.
fn parse_version(v: &str) -> (u32, u32) {
    let mut parts = v.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configuration_is_checked() {
        assert!(PgVectorConfig::new(0).is_err());
        assert!(PgVectorConfig::new(MAX_DIMENSIONS + 1).is_err());
        // Too wide for an approximate index, fine for an exact one.
        assert!(PgVectorConfig::new(3072).is_err());
        assert!(PgVectorConfig::new(8)
            .unwrap()
            .with_ann(AnnIndex::Exact)
            .is_ok());
        for bad in [
            "",
            "Chunks",
            "1chunks",
            "chunks; DROP TABLE x",
            "a-b",
            &"a".repeat(41),
        ] {
            assert!(
                PgVectorConfig::new(8).unwrap().with_table(bad).is_err(),
                "{bad}"
            );
        }
        assert!(PgVectorConfig::new(8).unwrap().with_table("_rag_2").is_ok());
        assert!(PgVectorConfig::new(8)
            .unwrap()
            .with_ann(AnnIndex::IvfFlat { lists: 0 })
            .is_err());
    }

    #[test]
    fn the_ddl_follows_the_configuration() {
        let c = PgVectorConfig::new(1024)
            .unwrap()
            .with_table("docs")
            .unwrap();
        let ddl = c.migrations().join(";\n");
        assert!(ddl.starts_with("CREATE EXTENSION IF NOT EXISTS vector"));
        assert!(ddl.contains("embedding     vector(1024) NOT NULL"));
        assert!(ddl.contains("source_writer text NOT NULL CHECK (source_writer <> '')"));
        assert!(ddl.contains("USING hnsw (embedding vector_cosine_ops)"));
        let ivf = c
            .clone()
            .with_ann(AnnIndex::IvfFlat { lists: 100 })
            .unwrap();
        assert!(ivf.migrations().join(";").contains("WITH (lists = 100)"));
        let exact = c.with_ann(AnnIndex::Exact).unwrap();
        assert_eq!(exact.migrations().len(), 3);
    }

    #[test]
    fn versions_compare_by_major_and_minor() {
        assert!(parse_version("0.8.1") >= MIN_PGVECTOR_FOR_ANN);
        assert!(parse_version("1.0.0") >= MIN_PGVECTOR_FOR_ANN);
        assert!(parse_version("0.7.4") < MIN_PGVECTOR_FOR_ANN);
        assert_eq!(parse_version("x"), (0, 0));
    }
}
