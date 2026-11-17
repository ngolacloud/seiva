# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the project is pre-1.0, the public API may change in a minor release.

## [Unreleased]

### Added

- Pluggable data layer: sqlx and SeaORM reference adapters alongside the default
  DORM adapter, selected at the composition root (ADR 0030).
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

[Unreleased]: https://github.com/ngolacloud/seiva/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ngolacloud/seiva/releases/tag/v0.1.0
