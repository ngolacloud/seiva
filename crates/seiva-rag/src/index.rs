//! The index port: chunks, the access filter, and the only search there is.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use seiva_identity::Principal;

use crate::RagError;

/// Where a document came from and who put it there (OWASP LLM04). Required at
/// ingestion: a chunk nobody can trace is not indexed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// The document's address (a URL, a path, a record id).
    pub uri: String,
    /// Who wrote or uploaded it (a subject, a system).
    pub writer: String,
}

/// Who may read a chunk: its tenant (`None` = shared by every tenant) and the
/// permission a reader must hold (`None` = anyone in that tenant).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Access {
    /// The tenant, or `None` for a document every tenant shares.
    pub tenant: Option<String>,
    /// The permission a reader must hold.
    pub requires: Option<String>,
}

/// One indexed piece of a document.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// `<document>#<ordinal>`.
    pub id: String,
    /// The document it belongs to.
    pub document: String,
    /// Its position in the document (from 0).
    pub ordinal: usize,
    /// The document's title, if any.
    pub title: Option<String>,
    /// The text.
    pub text: String,
    /// Who may read it.
    pub access: Access,
    /// Where it came from.
    pub source: Provenance,
    /// The embedding model that produced `vector`.
    pub model: String,
    /// The embedding.
    pub vector: Vec<f32>,
}

/// What a reader may see: their tenant and permissions. Built from the caller's
/// principal — never from a request parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessFilter {
    tenant: Option<String>,
    permissions: BTreeSet<String>,
}

impl AccessFilter {
    /// The filter of `principal`: chunks of their tenant (and shared ones) whose
    /// permission they hold. A principal with no tenant sees only shared chunks.
    pub fn of(principal: &Principal) -> Self {
        Self {
            tenant: principal.tenant.clone(),
            permissions: principal.permissions.clone(),
        }
    }

    /// Whether this reader may see a chunk with `access`.
    pub fn allows(&self, access: &Access) -> bool {
        let tenant_ok = match &access.tenant {
            None => true,
            Some(t) => self.tenant.as_deref() == Some(t.as_str()),
        };
        let permission_ok = access
            .requires
            .as_ref()
            .map_or(true, |p| self.permissions.contains(p));
        tenant_ok && permission_ok
    }

    /// The reader's tenant (for an adapter's `WHERE`).
    pub fn tenant(&self) -> Option<&str> {
        self.tenant.as_deref()
    }

    /// The reader's permissions (for an adapter's `WHERE`).
    pub fn permissions(&self) -> &BTreeSet<String> {
        &self.permissions
    }
}

/// One search. The filter is not optional: there is no way to search an index
/// without saying who is reading (OWASP LLM08).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchQuery<'a> {
    /// The query embedding.
    pub vector: &'a [f32],
    /// The model that embedded it (only chunks of the same model compare).
    pub model: &'a str,
    /// Who is reading.
    pub filter: &'a AccessFilter,
    /// How many hits, at most — counted among the chunks the reader may see.
    pub k: usize,
}

/// A chunk found by a search.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// The chunk (its vector included).
    pub chunk: Chunk,
    /// Cosine similarity to the query.
    pub score: f32,
}

/// Where chunks are kept. An adapter **must** apply the filter inside the query
/// (a pgvector `WHERE tenant = $1 AND …` before the `ORDER BY distance LIMIT k`),
/// so the top `k` are counted among visible chunks and an invisible chunk never
/// leaves the store.
#[async_trait]
pub trait VectorIndex: Send + Sync {
    /// Replace every chunk of `document` with `chunks` (an empty list deletes it).
    async fn replace_document(&self, document: &str, chunks: Vec<Chunk>) -> Result<(), RagError>;

    /// The `k` nearest chunks the reader may see, best first.
    async fn search(&self, query: &SearchQuery<'_>) -> Result<Vec<Hit>, RagError>;
}

/// An in-process [`VectorIndex`]: exact cosine search, filter first. For tests
/// and small corpora; one process only.
#[derive(Clone, Default)]
pub struct InMemoryIndex {
    documents: Arc<RwLock<BTreeMap<String, Vec<Chunk>>>>,
}

impl InMemoryIndex {
    /// Empty.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many chunks are kept.
    pub fn len(&self) -> usize {
        self.documents
            .read()
            .map(|d| d.values().map(Vec::len).sum())
            .unwrap_or(0)
    }

    /// Whether it is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl VectorIndex for InMemoryIndex {
    async fn replace_document(&self, document: &str, chunks: Vec<Chunk>) -> Result<(), RagError> {
        let mut docs = self
            .documents
            .write()
            .map_err(|_| RagError::Index("index poisoned".into()))?;
        if chunks.is_empty() {
            docs.remove(document);
        } else {
            docs.insert(document.to_owned(), chunks);
        }
        Ok(())
    }

    async fn search(&self, query: &SearchQuery<'_>) -> Result<Vec<Hit>, RagError> {
        let docs = self
            .documents
            .read()
            .map_err(|_| RagError::Index("index poisoned".into()))?;
        let mut hits: Vec<Hit> = docs
            .values()
            .flatten()
            // The filter comes first: an invisible chunk is never scored.
            .filter(|c| query.filter.allows(&c.access) && c.model == query.model)
            .map(|c| Hit {
                score: cosine(query.vector, &c.vector),
                chunk: c.clone(),
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(query.k);
        Ok(hits)
    }
}

/// Cosine similarity (0 for a zero vector or a length mismatch).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_follows_tenant_and_permission() {
        let ana = AccessFilter::of(&Principal::in_tenant("ana", "acme", ["docs.hr"]));
        let shared = Access::default();
        let acme = Access {
            tenant: Some("acme".into()),
            requires: None,
        };
        let acme_hr = Access {
            tenant: Some("acme".into()),
            requires: Some("docs.hr".into()),
        };
        let acme_board = Access {
            tenant: Some("acme".into()),
            requires: Some("docs.board".into()),
        };
        let zeta = Access {
            tenant: Some("zeta".into()),
            requires: None,
        };
        assert!(ana.allows(&shared) && ana.allows(&acme) && ana.allows(&acme_hr));
        assert!(!ana.allows(&acme_board) && !ana.allows(&zeta));
        let no_tenant = AccessFilter::of(&Principal::new("svc", Vec::<String>::new()));
        assert!(no_tenant.allows(&shared) && !no_tenant.allows(&acme));
    }

    #[test]
    fn cosine_is_safe_on_odd_input() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 0.0]), 0.0);
    }
}
