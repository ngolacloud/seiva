# seiva-llm

The provider-neutral language-model port of [Delonix Seiva](https://github.com/ngolacloud/seiva)
— phase G1 of [ADR 0048](https://github.com/ngolacloud/seiva/blob/main/docs/adr/0048-ai-gateway-and-llm-harness.md),
the AI gateway. **DORM-free and http-client-free**: the codecs talk to the network only
through an injected `HttpTransport` (the reqwest/rustls one is
[`seiva-llm-reqwest`](../seiva-llm-reqwest)).

A product never names a provider. It calls a **route**, and the router maps the route to
its upstreams (provider + model, in fallback order) while keeping the route's promise
about where data goes.

## What's in it

| Piece | What it does |
|-------|--------------|
| `ChatRequest`, `Message`, `Content`, `ToolSpec`, `ChatResponse`, `Usage`, `StreamEvent` | The neutral conversation: text, images, tool use and tool results, streaming deltas, usage with cached tokens counted apart. `max_tokens` is mandatory (OWASP LLM10). |
| `ChatProvider` / `ChatModel` / `Embedder` | The traits: adapters implement `ChatProvider`; a use case holds a `ChatModel` (a route). |
| `Router`, `Route`, `Residency` | The table is checked when built — an unknown provider, an `in_country` route with an `abroad` upstream, a route with neither `.requires(..)` nor `.public()` are refused. At run time: a time budget per attempt, retry of transient failures with the framework-wide backoff, fallback to the next upstream. |
| `CredentialSource`, `EnvCredentials`, `StaticCredentials`, `Secret` | Provider keys belong to the organisation, never to the caller. A `Secret` prints `***`; a credential header is marked sensitive; a provider that echoes the key in its error has it wiped. |
| `providers::anthropic::Anthropic` | The Messages API, whole and streamed. |
| `providers::openai::OpenAiCompatible` | Chat Completions and embeddings: OpenAI, Azure OpenAI, vLLM, Ollama, Gemini's compatible endpoint, or an OpenAI-compatible gateway already on the network. |

## Quick start

```rust
use std::sync::Arc;
use seiva_llm::providers::anthropic::Anthropic;
use seiva_llm::providers::openai::OpenAiCompatible;
use seiva_llm::{ChatModel, ChatRequest, Endpoint, EnvCredentials, Residency, Route, Router};
use seiva_llm_reqwest::ReqwestTransport;

let transport = Arc::new(ReqwestTransport::with_defaults()?);
let creds = Arc::new(EnvCredentials);

let router = Router::builder()
    .provider(Arc::new(Anthropic::new(transport.clone(), creds.clone())), Residency::Abroad)
    .provider(
        Arc::new(OpenAiCompatible::new(
            "local",
            Endpoint::allow_plaintext("http://10.0.0.20:11434/v1")?,
            transport,
            creds,
        )),
        Residency::InCountry,
    )
    // Personal data never leaves the country: this route can only reach `local`.
    .route(Route::new("hr-assistant", Residency::InCountry).requires("llm.hr").upstream("local", "llama3.3"))
    .route(
        Route::new("support-chat", Residency::Abroad)
            .requires("llm.support")
            .upstream("anthropic", "claude-sonnet-5-5")
            .upstream("local", "llama3.3"),
    )
    .build()?;

let chat = router.route("support-chat")?;
let answer = chat.complete(&ChatRequest::new(512).user("Olá")).await?;
```

In tests, `seiva-testing` (feature `llm`) provides `ScriptedModel` — a provider that plays
a script and records every request — and a deterministic `HashEmbedder`.

## The gateway

Identity, authorization of the route's permission, budget, masking, guardrails and audit
are the gateway stages of ADR 0048 §2, in [`seiva-ai-gateway`](../seiva-ai-gateway). A
product that calls a model on behalf of people goes through the gateway, not the router.

## License

MIT OR Apache-2.0.
