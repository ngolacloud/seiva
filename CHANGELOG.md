# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the project is pre-1.0, the public API may change in a minor release.

## [Unreleased]

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

[Unreleased]: https://github.com/ngolacloud/seiva/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/ngolacloud/seiva/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/ngolacloud/seiva/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ngolacloud/seiva/releases/tag/v0.1.0
