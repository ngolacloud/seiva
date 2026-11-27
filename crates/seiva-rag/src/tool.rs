//! A `search_documents` tool for `seiva-agent`.

use async_trait::async_trait;
use seiva::contract::ToolEffect;
use seiva_agent::{AgentCtx, Tool, ToolOutput};
use seiva_llm::ToolSpec;
use serde_json::{json, Value};

use crate::answer::{documents_block, Retriever};

/// Searches the documents the person the agent acts for may read, and returns
/// them numbered and labelled as untrusted data, with their sources.
pub struct SearchTool {
    retriever: Retriever,
    k: usize,
}

impl SearchTool {
    /// The `k` best chunks per search.
    pub fn new(retriever: Retriever, k: usize) -> Self {
        Self {
            retriever,
            k: k.max(1),
        }
    }
}

#[async_trait]
impl Tool for SearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "search_documents",
            "Search the organisation's documents the current person may read. Returns numbered excerpts with their \
             sources; cite them as [n].",
            json!({"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}),
        )
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Read
    }

    async fn call(&self, ctx: &AgentCtx<'_>, input: Value) -> ToolOutput {
        let Some(query) = input
            .get("query")
            .and_then(Value::as_str)
            .filter(|q| !q.trim().is_empty())
        else {
            return ToolOutput::error("`query` is required");
        };
        match self.retriever.search(ctx.caller, query, self.k).await {
            Ok(hits) if hits.is_empty() => ToolOutput::ok("No document you may read matches."),
            Ok(hits) => {
                let sources: Vec<String> = hits
                    .iter()
                    .enumerate()
                    .map(|(i, h)| format!("[{}] {}", i + 1, h.chunk.source.uri))
                    .collect();
                ToolOutput::ok(format!(
                    "{}\n\nSources:\n{}",
                    documents_block(&hits),
                    sources.join("\n")
                ))
            }
            Err(e) => ToolOutput::error(e.to_string()),
        }
    }
}
