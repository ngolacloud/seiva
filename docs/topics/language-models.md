# Language models

> **Status: phase G1 of [ADR 0048](../adr/0048-ai-gateway-and-llm-harness.md).** The
> model port, routes and provider codecs ship in `seiva-llm`; the gateway stages
> (identity, budget, masking, guardrails, audit) are phase G2.

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
