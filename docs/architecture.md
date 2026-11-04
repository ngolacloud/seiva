# Architecture

Delonix Seiva is a backend framework built on one idea: **a module owns its data
and exposes a single, machine-readable contract.** Everything else — the HTTP
routes, the gRPC service, the OpenAPI document, the permission checks — is
derived from that contract. This guide explains the layering, the contract, and
how the pieces fit.

## The dependency rule

A module is five crates, and the compiler enforces the direction of
dependencies — you cannot violate the architecture and still build:

```
         interface
         /        \
  infrastructure   application
         \        /      \
          domain          facade
                            |
                        (seiva-ports, delonix-seiva)
```

- **`domain`** — entities and business rules. Depends on nothing (not even the
  framework). Example: `notes_domain::Note`, `accounts_domain::User` with its
  username/credential rules.
- **`facade`** — the module's public, versioned contract: a trait annotated with
  `#[seiva::facade]` plus its DTOs. The DTOs carry no secrets and no persistence
  concerns. This is the *only* thing other modules and transports see.
- **`application`** — the use cases: a service that implements the facade trait
  over outbound **ports** (a repository it defines, plus the framework's
  cross-cutting ports). Transport-free and data-layer-free — it names no `axum`,
  no SQL, no DORM.
- **`infrastructure`** — concrete adapters for the application's ports (a
  repository). The default is in-memory and DORM-free; a DORM-backed adapter
  comes from `seiva-data-dorm` behind the `dorm` feature.
- **`interface`** — composition and transport wiring (the `#[seiva::controller]`
  lives here). Depends only on the facade in production code (ADR 0003);
  composition of the concrete service belongs to the app that runs it.

The `examples/notes` and `examples/accounts` modules are the canonical shape and
exactly what `seiva module new` scaffolds.

## The facade contract

`#[seiva::facade(version = "v1")]` keeps your trait as written and generates a
zero-sized `<Trait>Facade` type whose `DESCRIPTOR` is known at compile time:

```rust
pub struct FacadeDescriptor {
    pub name: &'static str,        // "Accounts"
    pub version: &'static str,     // "v1"
    pub methods: &'static [MethodDescriptor],
}
pub struct MethodDescriptor {
    pub name: &'static str,        // "create_user"
    pub requires: &'static str,    // "accounts.create"  (from #[requires(..)])
    pub params: &'static [Param],  // [{ name: "cmd", ty: "CreateUser" }]
    pub output: &'static str,      // "Result<UserDto, String>"
}
```

There is no reflection and no runtime registry: a missing method or permission is
a compile-time fact. The hand-written equivalent of what the macro generates is
documented in `delonix_seiva::contract`.

## Transports generated from the descriptor

One contract, three transports — none hand-maintained:

- **HTTP** (`seiva-http` + `#[seiva::controller]`): the macro generates
  `<trait>_router(service) -> axum::Router`, one route per method. The path
  version and the required permission are read from the descriptor (never
  hard-coded); the guard rejects a caller without the permission with an RFC 9457
  `application/problem+json`. Request bodies are validated (`Valid<T>`).
- **gRPC** (`seiva-grpc`): `proto_document(&FacadeDescriptor) -> String` renders
  a proto3 service via a documented, auditable Rust-type → proto-type map;
  framework context parameters are not wire fields; `Result<T, E>` is unwrapped
  to `T`. The optional `tonic` feature maps a facade error to `tonic::Status`.
- **OpenAPI** (`seiva-openapi`): `openapi_document(ApiInfo, &[FacadeDescriptor])`
  builds a valid OpenAPI 3.1 document — operations under their versioned paths,
  `x-seiva-requires` and bearer security for guarded methods, honest placeholder
  schemas for opaque types. Served live at `/openapi.json`.

The `seiva openapi` and `seiva proto` CLI commands print these for the built-in
demonstration facade today; a generated module emits its own via its interface
layer.

## The ports (`seiva-ports`)

The application layer depends only on this trait-only crate, so it is testable
with no server, broker or database (`seiva-testing` provides in-memory doubles):

- `Clock` — the time source.
- `IdGenerator` — stable 16-byte ids.
- `EventPublisher` — publish a domain event.
- `UnitOfWork` — **scope-based**: `transaction(|| async { ... })` runs the body
  in one transaction. With the DORM gateway this maps onto `with_session` +
  `db::atomic`, and `EventPublisher` writes to the **transactional outbox** in
  the *same* transaction, so the business change and the event commit together.
  A relay worker forwards the event to the broker afterwards.

## Command/query dispatch (`seiva-cqrs`)

The internal application bus: `Command`/`Query` traits, their handlers, and a
statically-wired `Mediator<H>` (monomorphised — a missing handler is a compile
error, not a runtime panic). Typed middleware wraps dispatch: `Instrument`
(counts dispatches) and `CacheQueries` (memoises a query, passes commands
through). This is the seam the facade's `#[cached]`/`#[policy]` attributes map
onto.

## The server (`seiva-server`)

The runtime a module installs — the "main":

- `serve(config, shutdown)` binds a listener, builds the app from
  `seiva_http::router` (controllers + `/openapi.json` + `/health`), flips health
  `Starting → Ready` once bound and `→ Draining` on the shutdown signal, and
  serves with `axum::serve(..).with_graceful_shutdown(..)`.
- `run_workers(shutdown)` drives background passes through the framework worker
  runtime; the DORM outbox relay pass is injected here behind the `dorm` feature.
- `ServerConfig` is read 12-factor (`seiva-config`): `SEIVA_HTTP_ADDR` > `PORT` >
  `127.0.0.1:8080`, plus `APP_ENV` and a guard that rejects dev shortcuts in
  production.

## Cross-cutting crates

- `seiva-messaging` — a CloudEvents envelope and `Publisher` backends (NATS, AMQP,
  RabbitMQ; Kafka), each behind a feature.
- `seiva-auth` — Argon2id password hashing in PHC format.
- `seiva-config` — typed 12-factor configuration with the `AppEnv` production
  guard.
- `seiva-webhooks` — Standard Webhooks signing/verification (HMAC-SHA256, a
  replay window, constant-time comparison).

## Fitness functions (`seiva lint`)

`delonix-seiva`'s `lint` module encodes the architecture as checks: the layer
dependency rule and the module `[[uses]]`/`[[allow]]` zero-trust contract. They
run in the CLI and are unit-tested in CI, so a drift from the rules fails the
build rather than shipping.

## DORM: default, optional, and isolated

DORM is the default data layer but enters through exactly one crate,
`seiva-data-dorm` (the Gateway pattern), behind the `dorm` feature. Switching the
data layer off is simply not compiling that crate. Because the gateway depends on
the **private** DORM repository by a pinned git revision, it and its companions
(`seiva-outbox-relay`, `seiva-rag-worker`, `seiva-mcp`) are **excluded from the
default workspace**: everything else builds and tests with no credentials.

A token-gated CI job (`dorm-gateway`) builds and tests those crates against the
private repo when a `DORM_TOKEN` secret is present, and is a green no-op without
it. The `dorm` feature is left unwired in the default build on purpose: the lint
job runs `cargo clippy --all-features`, so a feature that pulled DORM would need
the token — it activates once the secret exists.

## See also

- [Quickstart](quickstart.md) — scaffold, run, and call a service end to end.
- [`docs/adr/`](adr/) — the design decisions (foundations, the ORM boundary, the
  tx-model resolution, …).
