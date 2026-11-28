# Language models

> **Status: phases G1–G3 and G7 of [ADR 0048](../adr/0048-ai-gateway-and-llm-harness.md).**
> The model port, routes and provider codecs ship in `seiva-llm`; the gateway stages
> (identity, budget, masking, guardrails, audit) in `seiva-ai-gateway`, in-process or
> behind OpenAI- and Anthropic-compatible endpoints (the `apps/ai-gateway` binary), for
> chat and for embeddings.

A use case that needs a model depends on a **route**, never on a provider. The route
says which upstreams may serve it, in fallback order, and where the data may go.

## Routes and residency

```rust
use seiva_llm::{Residency, Route, Router};

let router = Router::builder()
    .provider(anthropic, Residency::Abroad)   // processes data outside the country
    .provider(local, Residency::InCountry)    // a model on the organisation's network
    .route(Route::new("hr-assistant", Residency::InCountry).requires("llm.hr").upstream("local", "llama3.3"))
    .route(
        Route::new("support-chat", Residency::Abroad)
            .requires("llm.support")
            .upstream("anthropic", "claude-sonnet-5-5")
            .upstream("local", "llama3.3"),
    )
    .build()?;
```

`build()` refuses a wrong table instead of failing at request time:

- an `in_country` route with an `abroad` upstream — personal data could leave the country;
- a route with neither `.requires("permission")` nor `.public()` — the same fail-closed
  rule as a facade method without `#[requires]` ([ADR 0019](../adr/0019-fail-closed-authorization.md));
- an unknown provider, a route with no upstream, a name used twice.

## Embedding routes

A route serves one kind of call: chat by default, or embeddings. A chat route never
answers an embedding call, nor the other way round, unless it was declared for both.

```rust
use seiva_llm::{Residency, Route, RouteKind, Router};

let router = Router::builder()
    .provider(openai.clone(), Residency::Abroad)
    .embedder("openai", openai, Residency::Abroad)      // the same provider, its embeddings side
    .embedder("local", local_embedder, Residency::InCountry)
    .route(
        Route::new("docs-embed", Residency::InCountry)
            .with_kind(RouteKind::Embedding)             // or RouteKind::ChatAndEmbedding
            .requires("llm.embed")
            .upstream("local", "bge-m3"),
    )
    .build()?;

let vectors = router.embed("docs-embed", &texts).await?;
```

An embedding route's upstreams must be registered with `.embedder(..)` (`build()` says so
otherwise), and the residency rule is the same. The router is itself an `Embedder` whose
`model` is the route, so an ingestor holds the router and names the route. For calls made
on behalf of people, embed through the gateway instead (below).

## Calling a model

```rust
use seiva_llm::{ChatModel, ChatRequest};

let chat = router.route("support-chat")?;          // a ChatModel
let answer = chat.complete(&ChatRequest::new(512).system("Be brief.").user("Olá")).await?;
println!("{} (served by {:?})", answer.text(), answer.upstream);
```

`max_tokens` is mandatory. `stream(..)` returns deltas and always ends with
`StreamEvent::Completed`, carrying the whole answer. Tools are `ToolSpec`s; the model's
calls come back as `Content::ToolUse`, and results go back as `Content::ToolResult`.

## Retry, timeouts and fallback

Each attempt has a time budget (`attempt_timeout`, 120 s by default; while streaming, the
wait for each event). Transient failures — a transport error, a timeout, `429`, `5xx`,
Anthropic's `529` — are retried on the same upstream with the framework-wide backoff
(`RetryPolicy`). A provider-specific failure (`401`, `403`, `404`) moves to the next
upstream without retrying. A malformed request (`400`, `422`) stops the route: it would
fail everywhere. A stream falls back only before its first event.

## Providers

| Provider | Constructor | Credential |
|---|---|---|
| Anthropic | `Anthropic::new(transport, creds)` | `ANTHROPIC_API_KEY` |
| OpenAI | `OpenAiCompatible::openai(transport, creds)` | `OPENAI_API_KEY` |
| Google Gemini | `OpenAiCompatible::gemini(transport, creds)` | `GEMINI_API_KEY` |
| vLLM, Ollama, Azure, an OpenAI-compatible gateway | `OpenAiCompatible::new(name, endpoint, transport, creds)` | optional, `.with_credential(..)` |

An endpoint is `https` unless the composition root opts a plaintext one in with
`Endpoint::allow_plaintext(..)` — for a model on a network the organisation controls.

## Credentials and the transport

Provider keys come from a `CredentialSource` (`EnvCredentials` by default). A `Secret`
prints `***`, the key travels on one header marked sensitive, and an error never carries
it — even when a provider echoes the request back.

The codecs link no HTTP client: they send through the `HttpTransport` port. In production
use `seiva-llm-reqwest`, which never follows a redirect (it would carry the key to another
host). That port is also where the guarded outbound client of
[ADR 0045](../adr/0045-egress-ssrf-guard.md) plugs in.

## The gateway: every call through the same stages

A product that calls a model **on behalf of people** goes through `seiva-ai-gateway`, not
straight to the router. The stages run in a fixed order and each one fails closed; every
outcome, refusals included, is audited.

```text
Identify → Authorize → Budget.reserve → Mask → Guard.input
        → model → Guard.output → Unmask → Budget.settle → Audit
```

```rust
use seiva_ai_gateway::{BudgetScope, Caller, Gateway, InMemoryBudget};

let gateway = Gateway::builder(router)
    .budget(Arc::new(InMemoryBudget::new(clock, Duration::from_secs(86_400)).per(BudgetScope::Principal, 200_000)))
    .audit(audit_sink)                              // the AuditSink of ADR 0013/0043
    .tool("customer_lookup", "customers.read")     // a tool the model may be offered, and who may offer it
    .area_in_country("hr")                         // HR data never goes to an abroad route
    .build()?;

let caller = Caller::new(principal).in_area("support").with_request_id(request_id);
let gated = gateway.complete(&caller, "support-chat", request).await?;
```

| Stage | What it does |
|---|---|
| Identify | refuses an anonymous principal |
| Authorize | the route's permission; an `in_country` area cannot use an `abroad` route; every tool offered must be registered and the caller must hold its permission |
| Budget | holds the request's size plus `max_tokens` in the principal's, the tenant's and the area's budget; a store outage stops the call |
| Mask | personal data becomes `‹TAX_ID_1›`, `‹EMAIL_1›`…; a credential refuses the request; tool output is wrapped in `<untrusted_data>` and the model told it is data |
| Guard (in/out) | your `Guardrail`s; built in: no credential in the answer, no call to a tool the model was not offered; `JsonOutput` and `SizeLimit` ship ready |
| Unmask | the placeholders come back for a caller entitled to them (`RevealPolicy`) |
| Settle, Audit | the real usage is counted; one record per call with counts and names, never values |

The built-in detectors cover the Angolan identity card, the NIF (after the word NIF),
IBANs (mod-97), payment cards (Luhn), e-mail, phone numbers and credentials; add your own
identifiers with `PatternDetector`. Masking is pattern-based: it reduces what a model sees,
it does not make a prompt anonymous. On an `abroad` route it always runs.

`stream(..)` holds text back by up to 256 characters so a credential is caught before it
reaches the caller, and never releases half a placeholder.

### Embeddings through the gateway

```rust
let gated = gateway.embed(&caller, "docs-embed", texts).await?;
let embedder = gateway.embedder(caller);   // an `Embedder` for seiva-rag, in-process
```

An embedding call goes through the same stages, minus the two that have nothing to work
on: Identify → Authorize (the route's permission, its **kind**, the caller's residency) →
Budget → Mask → Guard (in) → model → Settle → Audit. A vector carries no text, so there is
no output guard and nothing to unmask; the answer is checked instead (one vector per
input, one dimension, finite numbers).

- **Masking changes the vector.** It runs under the chat rule — always on an `abroad`
  route, on an `in_country` route unless `mask_in_country(false)` — so the model embeds
  `O NIF ‹TAX_ID_1›`, not the number, and a search for the exact value of a masked
  identifier will not find it. That is the price of not sending the value abroad. Where
  exact identifiers must be searchable, embed on an `in_country` route with in-country
  masking off. A credential is refused either way.
- **Guardrails** get `check_embedding_input(texts)`; by default it hands the texts to
  `check_input` as user messages, so a chat guardrail (a `SizeLimit`) covers embeddings
  unchanged.
- **Budget**: the texts' size (characters ÷ 4) is held; the provider's input tokens are
  settled — the estimate when the provider reports none.
- **Audit**: one `llm.embed` record — number of texts, upstream, dimension, masked counts;
  never a text or a vector.

## The gateway as a server

The same gateway serves OpenAI- and Anthropic-compatible endpoints (`seiva-ai-gateway`,
feature `server`), so a corporate chat, a RAG front end or a coding agent points its base
URL at it and inherits every stage. The runnable binary is `apps/ai-gateway`: one TOML
file declares providers, routes, the gateway keys (only their SHA-256) and the budget.

| Endpoint | For |
|---|---|
| `POST /v1/chat/completions` | OpenAI clients, whole or streamed |
| `POST /v1/messages`, `/v1/messages/count_tokens` | Anthropic clients, whole or streamed |
| `POST /v1/embeddings` | OpenAI embedding clients (`input` a string or strings; vectors as numbers, or base64 when asked) |
| `GET /v1/models` | the routes this caller may use, each with its `capabilities`: `chat`, `embeddings` |

The client sends a **route** as `model` and a **gateway key** — never a provider key — as
`Authorization: Bearer` or `x-api-key`; the key decides who the caller is and in which
area. Provider keys stay in the gateway's environment. A coding agent (Claude Code, an
IDE) offers the tools it runs itself: give its key the gateway's `any_tool` permission.

In the TOML file a route says `kind = "embedding"` (or `"chat+embedding"`); OpenAI,
Gemini and OpenAI-compatible providers serve both kinds, Anthropic only chat. Any OpenAI
embeddings client works against `/v1/embeddings` — `seiva-llm`'s own
`OpenAiCompatible` embedder included: base URL the gateway, credential a gateway key,
model a route. Token arrays as `input` are refused (the gateway masks text), and so is
`dimensions` (the route decides the model).

## Testing

```rust
use seiva_testing::llm::ScriptedModel;   // feature "llm"

let model = Arc::new(ScriptedModel::new("fake").reply_text("Olá!"));
// … build the router over `model`, run the use case …
assert_eq!(model.calls()[0].request.messages.len(), 1);
```

`ScriptedModel` plays a script (answers, tool calls, errors) and records every request,
so a test can assert on what the use case sent — and an extra, unexpected call fails.

## Related

- [ADR 0048 — AI gateway and LLM harness](../adr/0048-ai-gateway-and-llm-harness.md)
- [Configuration](configuration.md) · [Testing](testing.md) · [Security](security.md)

---

*Part of the [Seiva documentation](../index.md).*
