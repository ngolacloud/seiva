# seiva-rag

Retrieval with permissions, and answers that cite — phase G6 of
[ADR 0048](https://github.com/ngolacloud/seiva/blob/main/docs/adr/0048-ai-gateway-and-llm-harness.md).
DORM-free.

```rust
let index = Arc::new(InMemoryIndex::new());          // or the pgvector adapter
let ingestor = Ingestor::new(embedder.clone(), "embed-model", index.clone());
ingestor.ingest(&Document {
    id: "hr-leave-policy".into(),
    title: Some("Leave policy".into()),
    text,
    access: Access { tenant: Some("acme".into()), requires: Some("docs.hr".into()) },
    source: Provenance { uri: "https://intranet/hr/leave".into(), writer: "hr-portal".into() },
}).await?;

let retriever = Retriever::new(embedder, "embed-model", index);
let a = answer(&gateway, "docs", &caller, &retriever, "How early must I ask for leave?", 4, 512).await?;
if a.uncited { /* show it as unsupported */ }
```

| OWASP (2025) | How |
|---|---|
| LLM08 Vector and embedding weaknesses | the index port's only search takes an `AccessFilter` built from the caller's principal (tenant + permissions), applied inside the query: tenant A never retrieves tenant B's chunk, and `k` counts visible chunks |
| LLM04 Data and model poisoning | `Ingestor` refuses a document without a source and a writer; every chunk keeps both |
| LLM09 Misinformation | `answer` numbers the chunks, asks through the gateway, and checks the `[n]` markers: `uncited` marks an answer without a source, `unknown_markers` an invented one |
| LLM01 Prompt injection | retrieved text reaches the model wrapped as untrusted data, a closing tag inside escaped |

With the `agent` feature (default), `SearchTool` is a `search_documents` tool for
`seiva-agent` that searches as the person.

## License

MIT OR Apache-2.0.
