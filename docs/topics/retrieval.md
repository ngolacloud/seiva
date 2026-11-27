# Retrieval (RAG)

> **Status: phase G6 of [ADR 0048](../adr/0048-ai-gateway-and-llm-harness.md)**, in
> `seiva-rag`.

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

The embedding model sees the text: prefer an in-country embedder for personal data.

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
