# Recuperação (RAG)

> **Estado: fase G6 do [ADR 0048](../adr/0048-ai-gateway-and-llm-harness.md)**, no
> `seiva-rag`.

A geração aumentada por recuperação responde a partir dos documentos da própria
organização. Duas coisas a tornam segura para pôr à frente de pessoas: uma pesquisa que só
encontra o que o leitor pode ler, e uma resposta que mostra de onde vem.

## Indexar com proveniência

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

Um documento sem fonte e sem autor é recusado, e cada pedaço guarda os dois — um pedaço
envenenado pode ser seguido até à origem e retirado (OWASP LLM04). Indexar outra vez o mesmo
id substitui o documento; o `remove` apaga-o.

O modelo de embeddings vê o texto: para dados pessoais, prefere um modelo no país.

## Pesquisar como o leitor

```rust
let retriever = Retriever::new(embedder, "embed-model", index);
let hits = retriever.search(&caller, "How early must I ask for leave?", 4).await?;
```

O port do índice tem uma só pesquisa, e ela recebe um `AccessFilter` construído a partir do
principal de quem chama: os pedaços do seu tenant (ou partilhados, `tenant: None`) cujo
`requires` ele tem. Um adaptador aplica-o **dentro da consulta** — um `WHERE` do pgvector
antes do `ORDER BY … LIMIT k` —, por isso os `k` melhores contam-se entre os pedaços
visíveis e um pedaço invisível nunca sai da base (OWASP LLM08). Um principal sem tenant só vê
os documentos partilhados.

## Respostas com citações

```rust
let a = answer(&gateway, "docs", &caller, &retriever, "How early must I ask for leave?", 4, 512).await?;
for c in &a.citations { println!("[{}] {}", c.n, c.uri); }
if a.uncited { /* show the answer as unsupported */ }
```

Os pedaços vão numerados e marcados como dados não confiáveis, o modelo é instruído a
citá-los como `[n]` e a dizer quando não respondem, e a chamada passa pelo
[gateway](language-models.md) (mascaramento, orçamento, auditoria). Os marcadores da
resposta são verificados: `uncited` marca uma resposta sem fontes, `unknown_markers` lista
referências inventadas (OWASP LLM09). Quando nada do que o chamador pode ler corresponde,
nenhum modelo é chamado.

## Para agentes

O `SearchTool` é uma ferramenta `search_documents` para o [`seiva-agent`](agents.md):
pesquisa como a pessoa em nome de quem o agente age e devolve excertos numerados com as
fontes.

## Relacionadas

- [Agents](agents.md) · [Language models](language-models.md) · [Security](security.md)

---

*Parte da [documentação do Seiva](../index.md).*
