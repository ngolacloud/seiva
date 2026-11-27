//! Ingestion: documents into chunks, embedded and indexed — only with provenance.

use std::sync::Arc;

use seiva_llm::Embedder;

use crate::index::{Access, Chunk, Provenance, VectorIndex};
use crate::RagError;

/// A document to index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// A stable id; ingesting the same id again replaces the document.
    pub id: String,
    /// Its title, if any.
    pub title: Option<String>,
    /// The text.
    pub text: String,
    /// Who may read it.
    pub access: Access,
    /// Where it came from, and who put it there.
    pub source: Provenance,
}

/// How text is cut: paragraphs packed up to `max_chars`, a longer paragraph cut
/// at a word boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunking {
    /// The largest chunk, in characters.
    pub max_chars: usize,
}

impl Default for Chunking {
    fn default() -> Self {
        Self { max_chars: 1200 }
    }
}

impl Chunking {
    /// Cut `text` into chunks.
    pub fn split(&self, text: &str) -> Vec<String> {
        let max = self.max_chars.max(80);
        let mut pieces: Vec<String> = Vec::new();
        for para in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
            if para.chars().count() <= max {
                pieces.push(para.to_owned());
                continue;
            }
            // A long paragraph: cut at the last space before the limit.
            let mut rest = para;
            while rest.chars().count() > max {
                let limit = rest.char_indices().nth(max).map_or(rest.len(), |(i, _)| i);
                let cut = rest[..limit]
                    .rfind(char::is_whitespace)
                    .filter(|&i| i > 0)
                    .unwrap_or(limit);
                pieces.push(rest[..cut].trim().to_owned());
                rest = rest[cut..].trim_start();
            }
            if !rest.is_empty() {
                pieces.push(rest.to_owned());
            }
        }
        // Pack consecutive small pieces into chunks of at most `max`.
        let mut chunks: Vec<String> = Vec::new();
        for p in pieces {
            match chunks.last_mut() {
                Some(last) if last.chars().count() + 2 + p.chars().count() <= max => {
                    last.push_str("\n\n");
                    last.push_str(&p);
                }
                _ => chunks.push(p),
            }
        }
        chunks
    }
}

/// What ingesting one document did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestReport {
    /// The document.
    pub document: String,
    /// How many chunks it became.
    pub chunks: usize,
}

/// Cuts, embeds and indexes documents.
///
/// The embedding model sees the document text. A model outside the country gets
/// it in clear unless the deployment masks first — prefer an in-country embedder
/// for personal data.
pub struct Ingestor {
    embedder: Arc<dyn Embedder>,
    model: String,
    index: Arc<dyn VectorIndex>,
    chunking: Chunking,
}

impl Ingestor {
    /// An ingestor embedding with `model` of `embedder` into `index`.
    pub fn new(
        embedder: Arc<dyn Embedder>,
        model: impl Into<String>,
        index: Arc<dyn VectorIndex>,
    ) -> Self {
        Self {
            embedder,
            model: model.into(),
            index,
            chunking: Chunking::default(),
        }
    }

    /// Cut differently.
    #[must_use]
    pub fn with_chunking(mut self, chunking: Chunking) -> Self {
        self.chunking = chunking;
        self
    }

    /// Index `document`, replacing an earlier version with the same id. Refused
    /// without an id, a source and a writer (OWASP LLM04): a chunk nobody can
    /// trace back is not indexed.
    pub async fn ingest(&self, document: &Document) -> Result<IngestReport, RagError> {
        if document.id.trim().is_empty() {
            return Err(RagError::Invalid("a document needs an id".into()));
        }
        if document.source.uri.trim().is_empty() || document.source.writer.trim().is_empty() {
            return Err(RagError::NoProvenance(document.id.clone()));
        }
        if document
            .access
            .requires
            .as_deref()
            .is_some_and(|p| p.trim().is_empty())
        {
            return Err(RagError::Invalid(format!(
                "document `{}` has an empty permission",
                document.id
            )));
        }
        let texts = self.chunking.split(&document.text);
        if texts.is_empty() {
            return Err(RagError::Invalid(format!(
                "document `{}` has no text",
                document.id
            )));
        }
        let embeddings = self
            .embedder
            .embed(&self.model, &texts)
            .await
            .map_err(|e| RagError::Embedding(e.to_string()))?;
        if embeddings.vectors.len() != texts.len() {
            return Err(RagError::Embedding(format!(
                "expected {} vectors, got {}",
                texts.len(),
                embeddings.vectors.len()
            )));
        }
        let chunks: Vec<Chunk> = texts
            .into_iter()
            .zip(embeddings.vectors)
            .enumerate()
            .map(|(ordinal, (text, vector))| Chunk {
                id: format!("{}#{ordinal}", document.id),
                document: document.id.clone(),
                ordinal,
                title: document.title.clone(),
                text,
                access: document.access.clone(),
                source: document.source.clone(),
                model: self.model.clone(),
                vector,
            })
            .collect();
        let n = chunks.len();
        self.index.replace_document(&document.id, chunks).await?;
        Ok(IngestReport {
            document: document.id.clone(),
            chunks: n,
        })
    }

    /// Remove `document` from the index.
    pub async fn remove(&self, document: &str) -> Result<(), RagError> {
        self.index.replace_document(document, Vec::new()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraphs_are_packed_and_long_ones_cut_at_a_word() {
        let c = Chunking { max_chars: 80 };
        let text = format!("{}\n\nCurto.\n\nOutro curto.", "palavra ".repeat(30));
        let chunks = c.split(&text);
        assert!(
            chunks.iter().all(|ch| ch.chars().count() <= 80),
            "{chunks:?}"
        );
        assert!(
            chunks.iter().all(|ch| !ch.ends_with("palavr")),
            "never mid-word"
        );
        assert!(
            chunks.last().unwrap().contains("Curto.\n\nOutro curto."),
            "{chunks:?}"
        );
        assert!(Chunking::default().split("  \n\n ").is_empty());
    }
}
