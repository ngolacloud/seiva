# ADR 0048 — AI gateway and LLM harness: one controlled path between a product and any model

- Status: Proposed
- Date: 2026-11-27
- Deciders: NgolaCloud
- Follows: ADR 0006 (identity), ADR 0013 / 0043 (audit), ADR 0015 (distributed rate limit), ADR 0019 (fail-closed authorization), ADR 0020 (pipeline behaviors), ADR 0025 (WireSchema), ADR 0027 (observability), ADR 0038 (OAuth 2.1 server), ADR 0045 (egress guard)

## Context

Products built on Seiva now ask for language models: a support chat over the
company's documents, an agent that looks up a customer and opens a refund, a
developer tool that calls a model with code. Each of those sends data — often
personal data, often regulated — to a model that usually runs **outside the
organisation and outside the country**. Each one also lets a model *act*: call a
tool, read a record, write one.

When every product talks to the model on its own, every product must solve the
same problems again, and the weakest one is the organisation's exposure:

- **Whose request is it?** The provider key is a shared secret. A product that
  holds it can spend it on anyone's behalf; a leaked key is a leaked budget.
- **What leaves the building?** A prompt that carries an identity card number,
  a tax number or an IBAN sends it to a third party in another jurisdiction
  (Angola's personal-data law, Lei n.º 22/11, and similar laws elsewhere). So
  does a tool result that the agent pastes back into the conversation.
- **What may this caller use?** Which models, which tools, which data — and
  whether a given area may send data abroad at all.
- **What may the model do?** A model that can call `refund` can be talked into
  calling it. Prompt injection arrives through documents and tool results, not
  only through the user.
- **How much?** Tokens cost money and capacity; a loop or an abusive caller can
  burn both.
- **What happened?** Who asked what, which model answered, what was masked, which
  tool ran with what effect — with a record that holds up in an audit.
- **And if a control breaks?** If masking, the budget store or the audit sink is
  down, the safe answer is "the request does not leave", not "send it anyway".

The industry answer is an **AI gateway**: every client — chat, RAG, coding agent,
desktop app — goes through one service inside the organisation, which applies
identity, masking, permissions, guardrails, budget and audit, fails closed, and
only then calls the model. Tools reach internal systems through **MCP** behind the
same gateway, with the identity of the person who asked. Proxies such as LiteLLM
made this pattern popular. In short: *autonomy at the edge, control in the middle.*

Seiva already owns most of the control pieces: a principal for every request
(ADR 0006), permissions declared on every facade method and enforced fail-closed
(ADR 0019), a typed pipeline of behaviors around a use case (ADR 0020), audit
(ADR 0013/0043), rate limiting (ADR 0015), observability (ADR 0027), an OAuth 2.1
server (ADR 0038) and a guarded outbound client (ADR 0045). What it lacks is the
model itself as a port, the gateway stages that join those pieces on the path to
a model, and a way to expose module facades as agent tools.

The forces:

- **Take the idea, not the product.** The gateway must be Seiva's own code, under
  Seiva's identity, authorization and audit rules — not an external proxy whose
  rules the framework cannot see or enforce.
- **Provider-neutral.** Anthropic, OpenAI, Google, a model on the organisation's
  own GPUs, or another gateway already on the network — the domain and
  application code must not know which.
- **Same code, two shapes.** Like a module (monolith or microservice by
  composition), the gateway must run in-process in a product *and* as a
  standalone server that other clients point at.
- **Secure by default, extensible by design.** The basic controls ship on and
  tested; a developer adds a guardrail, a masking rule, a provider, a tool or an
  agent without forking the framework.
- **Measurable against a public standard.** The OWASP Top 10 for LLM
  Applications (2025) is the acceptance list; each risk is closed by a stage
  with a test that proves it.

## Decision

**Seiva gets a provider-neutral model port, an AI gateway built as a typed
pipeline of fail-closed stages, MCP tools generated from facades with the
caller's identity, and an agent harness the developer extends.** LiteLLM is the
inspiration, never a dependency: no LiteLLM library, proxy or image is linked or
shipped. A LiteLLM proxy (or any OpenAI-compatible gateway) the organisation
already runs is just another endpoint on the network — reached by the
OpenAI-compatible adapter through the egress guard, or placed in front of a Seiva
gateway as an ordinary client.

### 1. The model port — `seiva-llm` (DORM-free)

- Provider-neutral types: messages with roles and content parts (text, image,
  tool use, tool result), tool definitions whose input schema comes from
  `WireSchema` (ADR 0007/0025), streaming events, usage (input, output and
  cached tokens) and a stop reason.
- Traits: `ChatModel` (complete and stream) and `Embedder`. The RAG worker
  bridges `Embedder` to DORM's embedder, so one model registry serves both.
- **Routes, not raw models.** A product calls a *route* (`support-chat`,
  `code-review`), never a provider model id. A route names an ordered list of
  upstreams (provider + model) for fallback, a `residency` (`in_country` or
  `abroad`) and the permission a caller must hold to use it.
- Adapters behind cargo features: `anthropic`, `openai-compatible` (OpenAI, Azure
  OpenAI, vLLM, Ollama, an existing LiteLLM proxy…), `gemini`. Every adapter
  dials through the guarded client of ADR 0045, so a model endpoint is an
  allowlisted host; a model on the organisation's own network (a private IP) is
  an explicit allowlist entry, never an implicit bypass.
- Retries and fallback reuse `seiva-resilience`; a fallback never crosses a
  residency boundary (an `in_country` route cannot fall back to an `abroad`
  upstream).
- Provider credentials come from a `CredentialSource` (environment by default,
  12-factor; a Vault adapter behind a feature). They never reach the caller and
  never appear in a log, an error or an audit entry.
- `seiva-testing` gains a scripted fake model, so a use case that calls a model
  is unit-tested without a network.

### 2. The gateway — `seiva-ai-gateway`

Each control is a behavior in the sense of ADR 0020: typed, composed by nesting,
no runtime registry. The order for one model call is:

```text
Identify → Authorize → Budget.reserve → Mask → Guard.input
        → model (via route, egress guard)
        → Guard.output → Unmask → Budget.settle → Audit
```

- **Identify.** The caller is a `Principal` (ADR 0006): a user session, an OAuth
  client (ADR 0038, `client_credentials` for services) or a gateway API key that
  Seiva issues and stores hashed, like any token. A provider key is never a
  caller credential.
- **Authorize.** The principal must hold the route's permission; the tools offered
  to the model are the **intersection** of what the route allows and what the
  principal may call. An area whose data may not leave the country is limited to
  `in_country` routes. Deny by default (ADR 0019).
- **Budget.** A `BudgetStore` port reserves an estimate before the call and
  settles the real usage after it, per principal, per area and per tenant, for a
  period. Unlike the HTTP limiter (ADR 0015, which fails open), **a budget store
  outage fails closed**: an unmetered model call is spend nobody approved. An
  in-memory store ships; a Redis store follows ADR 0015's atomic pattern.
- **Mask.** Detectors replace sensitive values with typed placeholders
  (`‹ID_CARD_1›`, `‹TAX_ID_2›`, `‹IBAN_1›`) before anything leaves; the mapping
  lives in memory for that request or conversation only, never persisted in
  clear. Built-in detectors cover the Angolan identity card, NIF and IBAN
  (`AO06…`), payment cards (Luhn), e-mail, phone numbers and secrets (keys,
  tokens, connection strings); a product adds its own. On an `abroad` route,
  masking is mandatory and cannot be switched off by the product.
- **Mask in tools.** A tool result is masked before it is added to the
  conversation; a tool argument that contains a placeholder is unmasked **inside**
  the trust boundary just before the tool runs, so the tool sees the real value
  and the model never does.
- **Guard.** A `Guardrail` trait with input and output hooks. Built in: content
  from tools and documents is wrapped and labelled as untrusted data, never as
  instructions; a size limit; an output check that blocks a secret or unmasked
  personal data from reaching the caller; schema validation of structured output
  before it is used. A product adds its own (a classifier, a topic policy).
- **Unmask.** Placeholders in the answer are restored only for a caller entitled
  to the original value, otherwise they stay masked.
- **Audit.** One entry per call through `AuditSink`: principal, route, upstream
  model, tokens, cost, how many values were masked (never the values), guardrail
  decisions, tools called and their effect. Prompts and answers are not stored by
  default; a product may opt in to store the **masked** text. Writes and refunds
  use the fail-closed critical audit of ADR 0043. Spans follow the OpenTelemetry
  GenAI semantic conventions (ADR 0027).
- **Fail closed.** Any stage that errors — a detector, the budget store, a
  guardrail, the critical audit — stops the request; there is no "send it
  unmasked" path and no flag that turns this off globally.

**Two shapes, same code.** In-process, a product composes the stages around its
use case (`Gateway::new(route, stages…)`). As a server (`seiva-ai-gateway`
binary on `seiva-server`), it exposes an OpenAI-compatible
`/v1/chat/completions`, an Anthropic-compatible `/v1/messages` and `/v1/models`,
so a corporate chat, a RAG front end or a coding agent (Claude Code, an IDE such
as dxtutor-ide) points its base URL at the gateway and inherits every control.

### 3. MCP tools from facades

A facade method opts in to being an agent tool; nothing is exposed by default
(excessive agency, OWASP LLM06):

```rust
#[seiva::facade(version = "v1")]
pub trait Payments {
    /// Refund a captured payment, fully or partly.
    #[requires("payments.refund")]
    #[tool(effect = "write")]
    async fn refund(&self, ctx: &Ctx, cmd: RefundPayment) -> Result<RefundDto, String>;
}
```

- The descriptor that already drives HTTP, gRPC and OpenAPI drives the MCP tool:
  the name, the description from the doc comment and the input schema from
  `WireSchema`. `effect` is required (`read` or `write`): a tool's effect is a
  compile-time fact, not a guess.
- **On behalf of the person.** The MCP call carries the principal of the human
  who asked (token exchange, RFC 8693, through ADR 0038), never a broad agent
  token; `#[requires]` and the BOLA scope (ADR 0005) apply exactly as on HTTP.
- **Writes wait for a human by default.** A `write` tool returns a pending
  approval that the client confirms; a deployment may pre-approve named write
  tools for named principals, and that policy is audited.
- **A DORM-free MCP server.** A new `seiva-mcp-server` crate serves the generated
  tools over MCP's Streamable HTTP transport behind Seiva authentication, with the
  OAuth protected-resource metadata MCP clients expect. It implements the server
  subset Seiva needs (`initialize`, `tools/list`, `tools/call`, `ping`) pinned to
  one protocol version, with conformance tests. The existing `seiva-mcp` (DORM's
  database tools, excluded from the workspace) stays as it is and becomes one
  more tool source.

### 4. The agent harness — `seiva-agent`

A small loop the developer extends rather than rewrites: send → tool calls →
results → repeat, bounded by maximum steps, tokens, cost and wall time, with
cancellation. Tools are the generated facade tools, local Rust functions, or
tools from external MCP servers. Every call goes through the gateway stages. The
MCP client allowlists servers, pins each tool description by hash (a description
that changes needs approval again — the "rug pull") and treats every tool result
as untrusted data. Extension points are traits: `Tool`, `Guardrail`, `Detector`,
`Hook` (before and after a tool call) and the conversation memory store.

### 5. Retrieval with permissions

The RAG worker keeps indexing (`seiva-rag-worker`); retrieval filters chunks by
the caller's tenant and access **in the query**, never after it, and an answer
built from retrieved chunks carries their citations.

### Acceptance: OWASP Top 10 for LLM Applications (2025)

| Risk | Closed by | Proven by |
|---|---|---|
| LLM01 Prompt injection | untrusted-content labelling; tools limited to the principal's permissions; writes approved | an injected document that asks for a refund produces no write |
| LLM02 Sensitive information disclosure | Mask, Mask in tools, output guard | the fake model receives `‹TAX_ID_1›`, never the NIF |
| LLM03 Supply chain | `cargo deny`; upstreams and MCP servers allowlisted; tool descriptions pinned | a changed tool description is refused until approved |
| LLM04 Data and model poisoning | RAG ingestion records the source and the writer of each chunk | a chunk without provenance is not indexed |
| LLM05 Improper output handling | schema validation of structured output; output never interpolated into SQL, HTML or a shell | invalid structured output is rejected before use |
| LLM06 Excessive agency | opt-in `#[tool]`, required `effect`, on-behalf-of identity, write approval | an agent token cannot call a tool the user cannot |
| LLM07 System prompt leakage | no secrets or credentials in prompts; the output guard blocks them | a credential placed in a prompt is blocked |
| LLM08 Vector and embedding weaknesses | tenant and access filtering in the retrieval query | tenant A never retrieves tenant B's chunk |
| LLM09 Misinformation | citations on retrieved answers; the product decides what an uncited answer may do | an answer without sources is marked as such |
| LLM10 Unbounded consumption | Budget (fail closed), rate limit, step, token and time bounds | the call after the budget is spent is refused, and so is every call while the budget store is down |

The harness is also reviewed against the OWASP Top 10 for Agentic Applications,
the companion list for agents that plan and use tools.

## Alternatives considered

- **Depend on LiteLLM (the Python proxy as a sidecar, or a port of it).**
  Rejected. Its controls would live outside Seiva's principal, permissions and
  audit, so the framework could not prove what it enforces; it adds a second
  runtime and supply chain to every deployment; and the control that matters
  most here — masking with Angolan detectors, residency per area, writes on
  behalf of a person — would have to be rebuilt on its side anyway. An
  organisation that already runs it keeps it: the OpenAI-compatible adapter
  reaches it as an upstream.
- **Leave it to each product (only a client SDK).** Rejected: the same
  controls rewritten per product, and the weakest product is the organisation's
  exposure — the reasoning that made ADR 0045 one shared client.
- **Server only, no in-process mode.** Rejected: a monolith would pay a network
  hop and a second deployment for its own model calls, against Seiva's "same
  code, composition decides" principle.
- **A full MCP SDK for the server.** Not taken now. A full SDK brings its own
  transport and authentication model, which Seiva replaces anyway — the reason
  `seiva-mcp` does not use DORM's HTTP or OAuth. The tool subset is small and is
  conformance-tested; revisit if Seiva needs resources, sampling or elicitation.
- **Expose every facade method as a tool automatically.** Rejected: it hands a
  model every write the API has. Opt-in per method, with a declared effect, keeps
  the tool surface deliberate and reviewable.
- **Mask by deleting values instead of placeholders.** Rejected: the model loses
  the structure it needs ("the customer with `‹ID_CARD_1›`") and a tool cannot be
  called with the real value. Reversible placeholders inside the trust boundary
  keep both.

## Consequences

- **Easier.** A product calls a route and gets identity, masking, permissions,
  guardrails, budget and audit for free; switching provider is configuration; an
  existing facade becomes an agent tool with one attribute and keeps its
  permission and tenant scope; corporate chats and coding agents gain the same
  controls by pointing at the server.
- **Harder.** Every model call pays the stages (detectors are regular
  expressions and checksums over the prompt; the budget store is one round trip).
  Masking is pattern-based and will miss what it does not know: products must add
  detectors for their own identifiers, and masking reduces, not removes, what a
  model sees. Streaming must be unmasked and output-guarded incrementally, which
  holds back a few tokens at chunk boundaries.
- **Debt (dated 2026-11-27).** Built in phases, each with its OWASP tests:
  G1 `seiva-llm` + adapters + fake model; G2 the in-process gateway stages;
  G3 the server shape (OpenAI- and Anthropic-compatible); G4 `#[tool]`,
  `seiva-mcp-server`, on-behalf-of and write approval; G5 `seiva-agent` and the
  MCP client; G6 permission-aware retrieval and citations; G7 embeddings through
  the gateway and the pgvector index. Until G4, an agent can only use local tools.
- **Enforcement.** Each row of the OWASP table is a test in the crate that closes
  it; a fitness check that every `seiva-llm` adapter dials through the egress
  client; a check that a facade with `#[tool]` declares `effect`; the gateway's
  stage order is asserted by a test, like the transaction boundary in ADR 0020.

## Implementation notes

- **G4 (2026-11-27).** The MCP server authenticates the person's own token with the
  framework's `Authenticator` and runs each tool with that person's `Ctx`. Token exchange
  (RFC 8693) waits for the OAuth server of ADR 0038; until then the client presents the
  person's token directly — still never a broad agent token. The human approval of a write
  is an out-of-band confirmation: the host app calls `POST /mcp/approvals/{id}` with the
  person's token after showing them the exact arguments, and the model's retry carries the
  approval id; MCP elicitation, which needs a server-initiated stream, is not used.
- **G6 (2026-11-27).** Retrieval lives in the DORM-free `seiva-rag` crate behind a
  `VectorIndex` port whose only search takes the caller's access filter; the in-memory
  index ships, and the DORM/pgvector adapter (`seiva-rag-worker`) is to implement the same
  port. Embedding calls do not go through the gateway stages yet: the guide recommends an
  in-country embedder for personal data until they do.
- **G7 (2026-11-28).** Embeddings go through the gateway. A route declares its kind —
  `chat` by default, `embedding`, or both only when its configuration says so — and a call
  of the other kind is refused at Authorize; an embedding route's upstreams are providers
  registered as `Embedder`s, under the same residency rule. `Gateway::embed` in-process and
  `POST /v1/embeddings` on the server (OpenAI shape, vectors as numbers or base64) run
  Identify → Authorize (permission, kind, residency) → Budget.reserve → Mask → Guard.input →
  model → Budget.settle → Audit, each failing closed. **Masking follows the chat rule**
  (always on an `abroad` route, on an `in_country` one unless the deployment turns it off),
  so the vector is the masked text's: a search by the exact value of a masked identifier
  does not find it — the price of not sending the value. A deployment that needs exact
  identifiers in its vectors embeds on an `in_country` route with in-country masking off;
  a credential is refused either way. There is no output guard and nothing to unmask (a
  vector carries no text); the answer is checked for shape instead. The budget holds the
  texts' size and settles the provider's input tokens (the estimate when it reports none);
  the `llm.embed` audit record carries counts, the upstream and the dimension, never a
  text or a vector. `GET /v1/models` gives each route its `capabilities` (`chat`,
  `embeddings`), and the gateway is also an `Embedder` for one caller, so the `Ingestor`
  and the `Retriever` embed through it in-process exactly as `seiva-llm`'s
  OpenAI-compatible embedder does against the server. The pgvector index is the new
  `seiva-rag-pgvector` crate (sqlx, the client of the `seiva-data-sqlx` adapter; out of the
  default members, with its own CI job against `pgvector/pgvector:pg16`): the access filter
  is in the `WHERE` before `ORDER BY embedding <=> $q LIMIT k`, with pgvector's iterative
  index scans turned on (pgvector ≥ 0.8) so a selective filter still fills `k` through
  HNSW; a hit's score is `seiva_rag::cosine` of the stored vector — the in-memory index's
  function — and `replace_document` is one transaction under an advisory lock on the
  document. The DORM worker (`seiva-rag-worker`) may implement the same port later.

## Related

- [ADR 0005](0005-object-level-authorization-bola.md) · [ADR 0006](0006-identity-and-authentication.md) · [ADR 0013](0013-admin-mutation-audit.md) · [ADR 0015](0015-distributed-rate-limit.md) · [ADR 0019](0019-fail-closed-authorization.md) · [ADR 0020](0020-command-bus-and-pipeline-behaviors.md) · [ADR 0025](0025-wire-schema-serde-fidelity.md) · [ADR 0027](0027-native-observability.md) · [ADR 0038](0038-oauth-authz-server.md) · [ADR 0043](0043-append-only-audit.md) · [ADR 0045](0045-egress-ssrf-guard.md)

---

*Part of the [Seiva documentation](../index.md).*
