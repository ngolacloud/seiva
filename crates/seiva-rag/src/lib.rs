//! `seiva-rag` — retrieval with permissions, and answers that cite
//! (ADR 0048, phase G6).
//!
//! ```text
//! Ingestor ── provenance required ──▶ VectorIndex ◀── search(query, AccessFilter::of(caller)) ── Retriever
//!                                                                                                  │
//!                       answer(): numbered documents (untrusted) ─▶ gateway ─▶ model ─▶ [n] checked
//! ```
//!
//! - **Who reads decides what is found.** The index port has one search, and it
//!   takes an [`AccessFilter`] built from the caller's principal — their tenant
//!   and permissions. An adapter applies it inside the query, so the top `k` are
//!   counted among visible chunks and an invisible chunk never leaves the store
//!   (OWASP LLM08).
//! - **Provenance at ingestion.** A [`Document`] without a source and a writer is
//!   refused; every chunk keeps both (OWASP LLM04).
//! - **Answers cite.** [`answer()`](answer()) numbers the retrieved chunks, wraps each as
//!   untrusted data, asks through the gateway (masking, budget, audit), and checks
//!   the `[n]` markers of the reply: [`RagAnswer::uncited`] marks an answer with
//!   no source, [`RagAnswer::unknown_markers`] an invented one (OWASP LLM09).
//! - **For agents.** With the `agent` feature, `SearchTool` is a
//!   `search_documents` tool for `seiva-agent`, searching as the person.
//!
//! [`InMemoryIndex`] ships; `seiva-rag-pgvector` implements the same
//! [`VectorIndex`] on Postgres + pgvector (G7). Embed through the gateway
//! (`seiva_ai_gateway::Gateway::embedder`) so the texts are masked, budgeted and
//! audited like any model call.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod answer;
mod index;
mod ingest;
#[cfg(feature = "agent")]
mod tool;

pub use answer::{
    answer, cite, documents_block, Citation, RagAnswer, Retriever, CITE_INSTRUCTIONS,
};
pub use index::{
    cosine, Access, AccessFilter, Chunk, Hit, InMemoryIndex, Provenance, SearchQuery, VectorIndex,
};
pub use ingest::{Chunking, Document, IngestReport, Ingestor};
#[cfg(feature = "agent")]
pub use tool::SearchTool;

/// A retrieval failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RagError {
    /// A document without a source or a writer.
    #[error("document `{0}` has no provenance (a source and a writer are required)")]
    NoProvenance(String),
    /// Something else wrong with a document.
    #[error("invalid document: {0}")]
    Invalid(String),
    /// The embedding model failed.
    #[error("embedding: {0}")]
    Embedding(String),
    /// The index failed.
    #[error("index: {0}")]
    Index(String),
    /// The gateway refused or failed the model call.
    #[error(transparent)]
    Gateway(#[from] seiva_ai_gateway::GatewayError),
}
