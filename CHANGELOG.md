# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the project is pre-1.0, the public API may change in a minor release.

## [Unreleased]

## [0.2.3] - 2026-11-28

Published on crates.io — every framework crate at 0.2.3. New crates on crates.io
for the first time: `seiva-kms` (envelope encryption), `seiva-agent` and
`seiva-rag`. Projects with `"0.2"` requirements pick it up with `cargo update`.

### Added

- **Envelope encryption** (ADR 0049): the new `seiva-kms` crate, extracted from
  dxstorage. A `KeyWrapper` port for the guardian of the key-encryption key, with two
  adapters: `LocalKek` (feature `local`, on by default; one base64 key per line, the first
  wraps) and `VaultTransit` (feature `vault`; token, token file, AppRole or Kubernetes
  auth, an extra CA, `wait_ready`). A pure `KeyRing` seals `sv1:<id>:<base64>` with
  XChaCha20-Poly1305 bound to `Aad` (`org|purpose|record`) and opens dxstorage's `kms1:`
  only through `open_kms1`. Also `RingStore` (compare-and-swap) and `Reseal` ports,
  `MemoryRingStore`, and `RingKeeper` (load/boot with a deadline, re-wrap after a KEK
  rotation, rotate, retire). The new `kms-vault` CI job runs the live tests against a
  real Vault.

- **Retrieval with permissions** (ADR 0048, phase G6): the new `seiva-rag` crate — a
  `VectorIndex` port whose only search takes an `AccessFilter` built from the caller's
  principal (tenant and permissions, applied in the query, so `k` counts visible chunks),
  an `Ingestor` that refuses a document without a source and a writer and keeps both on
  every chunk, a `Retriever`, and `answer`, which asks through the gateway with the chunks
  numbered and labelled as untrusted and checks the reply's `[n]` citations (`uncited`,
  `unknown_markers`). `InMemoryIndex` ships; `SearchTool` (feature `agent`) is a
  `search_documents` tool for `seiva-agent`.

- **Agent harness** (ADR 0048, phase G5): the new `seiva-agent` crate — a bounded loop
  (steps, tokens, wall time, per-tool timeout, `Cancel`) whose every model call goes
  through the AI gateway; tools from functions (`FnTool`), module facades in-process
  (`facades::tools`, the person's `#[requires]` checked) and allowlisted MCP servers
  (`mcp::tools`: JSON or SSE responses, sessions, each tool's description and schema
  pinned by hash — a new or changed tool is withheld until approved); writes only through
  an `Approver` (`DenyWrites` by default); `Hook`s before and after each call; `Memory`
  kept per principal. `seiva-mcp-server`'s `ToolSet` gains `invoke` for in-process calls.

### Changed

- `seiva_llm::HttpResponse` carries the response headers (`headers`, `header(..)`,
  `with_header(..)`) and is `#[non_exhaustive]`: build it with `new`/`from_bytes`. The
  reqwest transport fills them in. A custom `HttpTransport` written against 0.2.2 that
  built `HttpResponse { status, body }` by struct literal must switch to the constructors.

## [0.2.2] - 2026-11-27

Published on crates.io — every framework crate at 0.2.2, including the CLI and
the umbrella crate `seiva` (the successor of `delonix-seiva`) for the first time.
Install the CLI with `cargo install seiva-cli`; generated projects resolve their
`"0.2"` requirements from crates.io.
`seiva-mcp-server` 0.2.2 is published from the follow-up commit that shortens one
of its crates.io keywords (`model-context-protocol` is over the 20-character
limit) — metadata only, the same code as the `v0.2.2` tag.

### Added

- CLI: dynamic shell completion — `seiva completion <bash|zsh|fish|elvish|powershell>`
  prints a script that calls back into `seiva` on every TAB, completing commands,
  flags and values: `--kind`, `--data`, `--backend`, the known extensions, and
  the current project's modules (`module split` offers the modules not yet split,
  `module merge` the split ones).
- CLI: `seiva --version` (and the new `seiva version`) is a build card — commit
  and distance from the last release tag, build date, rustc, the linked data
  layer and demo commands, the framework version new projects are scaffolded on,
  how a project is shaped, and a get-started list. `-V` / `version --short` keep
  the one-line `seiva <version>` for scripts.
- **Language models** (ADR 0048, phase G1): the new `seiva-llm` crate — a
  provider-neutral model port (`ChatProvider`, `ChatModel`, `Embedder`; messages,
  tool use, streaming, normalised usage), routes with data residency that are
  checked when the router is built (an `in_country` route cannot reach an
  `abroad` upstream; a route needs `.requires(..)` or `.public()`), retry,
  per-attempt timeouts and fallback, a `CredentialSource` whose `Secret` never
  prints, and the Anthropic and OpenAI-compatible codecs (OpenAI, Azure, vLLM,
  Ollama, Gemini's compatible endpoint). The codecs talk through an
  `HttpTransport` port and link no HTTP client; the reqwest/rustls transport is
  the new `seiva-llm-reqwest` crate (excluded from the workspace, own CI job).
  `seiva-testing` gains `llm::ScriptedModel` and `llm::HashEmbedder` behind the
  `llm` feature.
- **AI gateway** (ADR 0048, phase G2): the new `seiva-ai-gateway` crate runs every model
  call through the same stages, in a fixed order, each failing closed — identity, the
  route's permission and residency per area, the tools offered checked against the
  caller's permissions, a token budget per principal, tenant and area (a store outage
  stops the call), masking of personal data into typed placeholders (built-in detectors
  for the Angolan identity card and NIF, IBANs, payment cards, e-mail, phones), refusal of
  credentials, tool output labelled as untrusted, input and output guardrails (`JsonOutput`,
  `SizeLimit`, and built-in credential and tool-policy checks), unmasking by a reveal
  policy, and one audit record per call — refusals included — that never carries a value.
  `stream(..)` holds text back so a credential is caught before it is released.
- **AI gateway server** (ADR 0048, phase G3): `seiva-ai-gateway`'s `server` feature serves
  OpenAI-compatible `POST /v1/chat/completions`, Anthropic-compatible `POST /v1/messages`
  and `/v1/messages/count_tokens`, and `GET /v1/models` (the routes the caller may use),
  whole or streamed, authenticated by gateway keys stored only as SHA-256; errors in each
  client's own shape. The gateway gains `any_tool(permission)` for coding agents whose
  tools run on the client. The new `apps/ai-gateway` app is the runnable binary
  (`seiva-ai-gateway serve | check | hash-key`): one TOML file, `seiva-server` with health
  probes, a JSON-lines audit record, model calls over `seiva-llm-reqwest`.
- **Agent tools over MCP** (ADR 0048, phase G4): `#[tool(effect = "read" | "write")]` on a
  facade method opts it in as an agent tool (the effect and a description are required,
  and a write tool cannot be `#[public]` — compile errors otherwise); `Facade::TOOLS`
  lists them; `#[seiva::tools]` on the impl generates the dispatcher, like
  `#[seiva::controller]`. The new `seiva-mcp-server` crate serves them over MCP's
  Streamable HTTP: the caller is the person whose token is presented, each tool's
  `#[requires]` is enforced and the method runs with their `Ctx`; a write waits for that
  person's approval (`/mcp/approvals/{id}`) and runs once; every call is audited, a write
  before it runs; `Origin` is checked and a 401 points to the RFC 9728 metadata.

### Changed

- CLI help is grouped and semantic, the same mechanism as the `delonix` CLI:
  `seiva --help` lists Project / Development / Contract / Quality / Tooling
  sections and ends with the usual workflow; groups are sectioned too (`seiva db
  --help`: Setup / Migration / Data). One-line summaries on `-h`; `--help` adds
  EXAMPLES and SEE ALSO for every command, from one tested table.
- `seiva::scaffold` exposes `FRAMEWORK_VERSION` and the `EXTENSIONS` list.

### Fixed

- `seiva-http` (`mtls`): the listener advertised ALPN `h2` but `axum::serve` was
  built without HTTP/2, so a client that negotiated `h2` — reqwest does by
  default — completed the TLS handshake and was then dropped with no response.
  The `mtls` feature now turns on `axum/http2`; both advertised protocols are
  served, and a test drives a request over a negotiated `h2` connection. A
  service that worked around it by enabling axum's `http2` itself can drop that.

## [0.2.1] - 2026-11-26

Tagged and released on GitHub; the crates are not republished on crates.io. A
project that pins the framework by git tag moves from `tag = "v0.2.0"` to
`tag = "v0.2.1"`; the `"0.2"` version requirements are unchanged. Install the
CLI from the tag:
`cargo install --git https://github.com/ngolacloud/seiva --tag v0.2.1 seiva-cli`.

### Added

- **Mutual TLS** (ADR 0047), opt-in behind the `mtls` feature of `seiva-http`
  and `seiva-server`. `MtlsListener` terminates TLS 1.3 and verifies client
  certificates against the deployment's CA, with optional CRLs, a handshake
  timeout and a cap on pending handshakes. `AuthConfig::with_client_certificates`
  maps the verified certificate to a `Principal` through the existing `authn`
  middleware: it is checked first, and a certificate sent together with a header
  credential gets `401`. `serve_router_mtls` and `MtlsFiles` (`SEIVA_TLS_*`)
  wire it into the runtime. The default build is unchanged.

## [0.2.0] - 2026-11-26

Tagged and released on GitHub; the crates are not yet republished on crates.io
(the 0.1.0 library crates there are unchanged). Install the CLI from the tag:
`cargo install --git https://github.com/ngolacloud/seiva --tag v0.2.0 seiva-cli`.

### Breaking

- The umbrella crate is renamed **`delonix-seiva` → `seiva`**. Generated projects
  now depend on `seiva = "0.2"` (and the other framework crates on `"0.2"`).
- The worker runtime moved to the new **`seiva-worker`** crate and the
  architecture lint to the new **`cargo-seiva`** crate (`cargo seiva lint`).
- A module's repository port lives in its `domain` crate (classic DDD), re-exported
  by `application` (F4).
- Tenant scope is mandatory: `Scope` is required by construction on the unit of
  work, and `Ctx::scope` fails closed (SG1, ADR 0029).

### Added

- Pluggable data layer: sqlx and SeaORM reference adapters alongside the default
  DORM adapter, selected at the composition root, with a shared ambient-transaction
  seam (`seiva-sql-ambient-tx`) and a facade-neutrality fitness function
  (ADR 0030).
- Events: the `seiva-events` crate — `EventSubscriber`, `SubscriberRegistry`,
  inbox-deduplicated dispatch, an in-process driver and background worker with
  retry/back-off and dead-lettering, and a typed, versioned domain-event contract
  with a link-time catalogue (ADR 0021, ADR 0046). `apps/shop-api` routes
  `orders.placed` → `inventory.reserved` live.
- CQRS pipeline behaviours: `Validate`, `Transactional`, `Authorize`,
  `Idempotent`, with `orders.place` dispatched through the command bus (ADR 0020).
- Admin: the opt-in dxjs console at `/admin/dx` (re-vendored from dxjs 1.2.0: the
  transport port, structured `SeivaError`, `@dxjs/ui` v2), `GET /admin/audit` over
  a new `AuditLog` port, and fail-closed `ListQuery` allow-list validation.
- HTTP: per-route load control (timeout → 504, backpressure → 503), Swagger UI
  and ReDoc next to Scalar, app-level API-key config, opaque `X-API-Key`
  authenticator next to bearer JWT.
- New crates: `seiva-contract` (transport-free wire contract), `seiva-resilience`
  (retry policy, plus a Helm chart and runbooks), `seiva-observability` (opt-in
  Prometheus `/metrics`), `seiva-webhook-delivery` and the `seiva-jobs` delivery
  composition root (ADR 0034), `WireId` for wire ids as strings (ADR 0018).
- Auth: pluggable password hashers and a reference credential flow (ADR 0017).
- CLI/scaffold: `module split` / `merge` (monolith ↔ microservice), project kinds,
  seed/demo CSV, use cases with transaction + event, a `list` method across the
  vertical, and a typed DORM adapter on `--data dorm`.
- Examples: the `polls` module and Django-polls tutorial, the five-module `shop`
  with `apps/shop-api`, and a dxjs storefront.
- Docs: bilingual site (EN + pt_AO), ADRs 0016–0046, architecture diagrams.
- `keywords` and `categories` metadata on the core crates for crates.io
  discoverability.
- Community files: `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, this changelog, and
  issue/PR templates.
- README install section (crates.io library crates + the git-installed CLI),
  badges, and a Rust-to-Rust comparison (axum/actix/Poem/Loco).

## [0.1.0] - 2026-11-05

Initial public release. The library crates are published on crates.io under a
dual MIT/Apache licence.

### Added

- Facade/controller/CQRS core: one `#[seiva::facade]` contract drives the HTTP
  controller, the gRPC proto, and the OpenAPI 3.1 document — no reflection.
- `seiva-server` runtime: facade-driven HTTP app, worker host, 12-factor config,
  graceful shutdown, `/health/live` + `/health/ready`.
- `seiva` CLI: project/module scaffolding, dev loop (`start --watch`),
  architecture fitness lint, `db init/dev/migrate/studio/pull`, `user create`.
- Security by default: verified JWT/OIDC + API-key auth, Argon2id password
  hashing, object-level authorization (`Scoped<T>`), secure headers, CORS,
  rate limiting (in-process + Redis).
- Observability: Prometheus `/metrics`, OpenTelemetry tracing, structured JSON
  logs (ADR 0027).
- Consistency primitives: transactional `UnitOfWork`, outbox relay, idempotency
  store, in-process event choreography.
- Two reference five-layer modules (`notes`, `accounts`) and a shop tutorial.

[Unreleased]: https://github.com/ngolacloud/seiva/compare/v0.2.3...HEAD
[0.2.3]: https://github.com/ngolacloud/seiva/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/ngolacloud/seiva/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/ngolacloud/seiva/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/ngolacloud/seiva/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ngolacloud/seiva/releases/tag/v0.1.0
