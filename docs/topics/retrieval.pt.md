# Recuperação (RAG)

> **Estado: fases G6 e G7 do [ADR 0048](../adr/0048-ai-gateway-and-llm-harness.md)**, no
> `seiva-rag` e no `seiva-rag-pgvector`.

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

O modelo de embeddings vê o texto. Faz os embeddings pelo
[gateway](language-models.md#embeddings-pelo-gateway) — `gateway.embedder(caller)`
in-process, ou o embedder compatível com OpenAI do `seiva-llm` apontado para o servidor do
gateway — e os textos são mascarados, contados no orçamento e auditados como qualquer
chamada a um modelo. O mascaramento muda o que é transformado em vector (`‹TAX_ID_1›`, não
o número), por isso uma pesquisa pelo valor exacto de um identificador mascarado não o
encontra; onde isso importe, faz os embeddings numa rota `in_country` com o mascaramento no
país desligado.

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

## Em Postgres: pgvector

```rust
use seiva_rag_pgvector::{sqlx::PgPool, AnnIndex, PgVectorConfig, PgVectorIndex};

let pool = PgPool::connect(&database_url).await?;
let index = PgVectorIndex::new(pool, PgVectorConfig::new(1024)?);   // HNSW, cosseno
index.migrate().await?;   // CREATE EXTENSION IF NOT EXISTS vector, a tabela, os índices
let index: Arc<dyn VectorIndex> = Arc::new(index);
```

O `seiva-rag-pgvector` guarda os pedaços numa tabela (colunas de proveniência `NOT NULL`, a
dimensão da coluna verificada em cada escrita e consulta) e põe o filtro do leitor no
`WHERE` do SQL, antes do `ORDER BY embedding <=> $query LIMIT k`. Com um índice aproximado
liga as pesquisas iterativas de índice do pgvector (pgvector 0.8 ou mais recente), para que
um filtro selectivo — um tenant entre muitos — continue a encher os `k` em vez de devolver
menos linhas; o `AnnIndex::Exact` dispensa o índice aproximado para resultados exactos, o
`AnnIndex::IvfFlat` constrói IVFFlat em vez de HNSW. As pontuações são o mesmo cosseno do
`InMemoryIndex`, e o `replace_document` é uma só transacção, por isso um leitor vê uma
versão do documento ou a outra. O `migrate` recusa uma tabela de outra dimensão em vez de a
alterar; o DDL também está disponível (`PgVectorConfig::migrations`) para uma instalação
que corre as suas próprias migrações. O crate liga o sqlx, por isso fica fora da build por
omissão do Seiva.

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
