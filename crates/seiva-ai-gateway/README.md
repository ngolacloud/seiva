# seiva-ai-gateway

The AI gateway of [Delonix Seiva](https://github.com/ngolacloud/seiva) — phase G2 of
[ADR 0048](https://github.com/ngolacloud/seiva/blob/main/docs/adr/0048-ai-gateway-and-llm-harness.md).
Every model call a product makes on behalf of people goes through the same stages, in a
fixed order, each one failing closed:

```text
Identify → Authorize → Budget.reserve → Mask → Guard.input
        → model (a seiva-llm route)
        → Guard.output → Unmask → Budget.settle → Audit
```

| Stage | What it does | OWASP LLM Top 10 (2025) |
|---|---|---|
| Identify | an authenticated `Principal` (ADR 0006) | — |
| Authorize | the route's permission; residency per area; every tool offered registered and permitted | LLM06 |
| Budget | tokens per principal, tenant and area; a store outage stops the call | LLM10 |
| Mask | personal data → `‹TAX_ID_1›`; a credential refuses the request; tool output labelled untrusted | LLM01, LLM02, LLM07 |
| Guard | your guardrails; built in: no credential in the answer, no call to a tool not offered | LLM02, LLM05, LLM06 |
| Unmask | placeholders restored for a caller entitled to them | LLM02 |
| Settle, Audit | real usage counted; one record per call, refusals too, never a value | — |

The order is fixed (a correctness invariant, asserted by a test); what each stage does is
pluggable: `Detector`, `Guardrail`, `BudgetStore`, and the `AuditSink` of `seiva-ports`.

**Embeddings** (G7) take the same path: `Gateway::embed` (or `gateway.embedder(caller)`,
an `Embedder` for `seiva-rag`) runs Identify → Authorize (permission, the route's kind,
residency) → Budget → Mask → Guard (in) → model → Settle → Audit. A chat route is not an
embedding route unless declared for both. Masking follows the chat rule, so the vector is
the masked text's (a search by a masked identifier's exact value will not find it; embed
on an `in_country` route with in-country masking off where that matters). The audit
record (`llm.embed`) keeps counts, the upstream and the dimension — never a text or a
vector.
Built-in detectors: the Angolan identity card and NIF, IBANs (mod-97), payment cards
(Luhn), e-mail, phone numbers, credentials. A gateway is not built without a budget store
and an audit sink.

## The server shape (feature `server`)

`server::router(ServerState)` serves `POST /v1/chat/completions` (OpenAI),
`POST /v1/messages` and `/v1/messages/count_tokens` (Anthropic), `POST /v1/embeddings`
(OpenAI, vectors as numbers or base64) and `GET /v1/models` (the routes the caller may
use, readable by both clients, each with its `capabilities`: `chat`, `embeddings`), whole
or streamed. The
client's `model` is a route; the client authenticates with a gateway key
(`Authorization: Bearer` or `x-api-key`) stored only as its SHA-256 (`GatewayKeys`),
which decides the principal and the area. Errors come back in the client's own shape.
The runnable binary is `apps/ai-gateway`. See the guide: `docs/topics/language-models.md`.

## License

MIT OR Apache-2.0.
