# Delonix Seiva

[![crates.io](https://img.shields.io/crates/v/delonix-seiva.svg)](https://crates.io/crates/delonix-seiva)
[![docs.rs](https://img.shields.io/docsrs/delonix-seiva)](https://docs.rs/delonix-seiva)
[![CI](https://github.com/ngolacloud/seiva/actions/workflows/ci.yml/badge.svg)](https://github.com/ngolacloud/seiva/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/delonix-seiva.svg)](#license)
[![MSRV 1.75+](https://img.shields.io/badge/MSRV-1.75%2B-blue.svg)](#install)

A Rust backend framework for robust, high-criticality systems — backend only,
ergonomic and batteries-included, a production- and cluster-ready server, and a
**data layer you choose**: [DORM](https://github.com/ngolacloud/dorm) by default,
or sqlx / SeaORM if that is what you already run. Seiva is the full-stack Rust
framework that does not wed you to an ORM.

Seiva is part of the Delonix family: **Delonix Engine** (containers and
microVMs), **DORM** (the ORM), and **Delonix Seiva** (this framework).

## What makes Seiva different

- A project is made of **modules that own their data**. There is no business
  code outside a module.
- The **same code** runs as a modular monolith or as a microservice, by
  configuration alone — no rewrite.
- **Clean Architecture enforced by the compiler**: five crates per module
  (`domain`, `application`, `facade`, `infrastructure`, `interface`), with the
  dependency rule imposed by the crate graph, not by convention.
- One **machine-readable contract per module** (`#[seiva::facade]`): the HTTP
  controller, the gRPC `.proto` and the OpenAPI 3.1 document are all generated
  from it — no reflection, all known at compile time.
- Zero trust between modules, even in the same process: every facade method
  declares the permission a caller must hold, and the transport enforces it.
- **The data layer is pluggable, by contract, not by accident.** Your domain and
  application code target `seiva-ports`, never an ORM. DORM is the default adapter;
  sqlx and SeaORM are first-class reference adapters. Bring the data layer you
  already run (ADR 0030).

## A taste

```rust
// A module's public contract — the single source of truth.
#[seiva::facade(version = "v1")]
pub trait Accounts {
    #[requires("accounts.create")]
    async fn create_user(&self, cmd: CreateUser) -> Result<UserDto, String>;
}
```

From that one trait, Seiva derives a descriptor that drives:

- an **HTTP controller** — `#[seiva::controller]` generates a facade-driven
  `axum` router (`POST /v1/accounts`), guarded by `accounts.create`;
- a **gRPC service** — `seiva proto` renders the proto3 document;
- an **OpenAPI 3.1** document — `seiva openapi`, served live at `/openapi.json`.

```console
$ seiva project new shop && cd shop
$ seiva module new catalog        # scaffolds the five layers, transport-wired
$ seiva-server                    # boots the HTTP app + workers (12-factor)
```

See the **[Quickstart](docs/quickstart.md)** to go end to end, and the
**[Architecture guide](docs/architecture.md)** for the layering and every crate's
role.

## Install

The library crates are on **crates.io** (dual MIT/Apache, MSRV 1.75). Add the
pieces a service needs — the framework core, the HTTP transport and the runtime:

```sh
cargo add delonix-seiva seiva-http seiva-server
# the trait-only data boundary + in-memory doubles for tests
cargo add seiva-ports --dev seiva-testing
```

The **`seiva` CLI** (scaffolds projects/modules, runs the dev loop, lints the
architecture) is not yet on crates.io — its demo commands depend on in-tree
example modules. Install it from git meanwhile:

```sh
cargo install --git https://github.com/ngolacloud/seiva seiva-cli
seiva project new shop && cd shop   # scaffold a project
```

Generated projects depend only on the published library crates, never on the
CLI, so this does not affect them. See [docs/publishing.md](docs/publishing.md)
for the per-crate publish status.

## Bring your own data layer — DORM is the default, not a requirement

The whole data-layer surface is a set of traits in `seiva-ports` (`UnitOfWork`,
`EventPublisher`, `IdGenerator`, `Clock`, `IdempotencyStore`, `AuditSink`,
`RateLimitStore`) plus the repository port each module declares in its own
`domain`. **No ORM type ever crosses that boundary** — so the adapter underneath
is a choice you make at the composition root, not a decision the framework makes
for you:

| Adapter | Feature | You get | You bring |
|---------|---------|---------|-----------|
| **DORM** (default) | `dorm` | migrations, transactional outbox, admin studio, per-module schema — batteries included | nothing; shortest path from zero |
| **sqlx** | `sqlx` | the ports over `sqlx::Transaction` | your schema + `sqlx migrate` |
| **SeaORM** | `seaorm` | the ports over SeaORM's `transaction()` | your entities + `sea-orm-cli` |

Already running sqlx or SeaORM in production? Adopting Seiva does not mean
rewriting persistence — implement the ports, keep your schema and queries. The
design is **ADR 0030**; `seiva-testing` ships an in-memory adapter that proves the
same contract.

The framework **core never depends on DORM** — or on any ORM. DORM enters only
through the `seiva-data-dorm` gateway behind the `dorm` feature, and the
dependency is one-way (Seiva → DORM, never the reverse). Everything in this
workspace builds and tests **without** the private DORM repository; the
DORM-coupled crates are a separate, token-gated build (see the architecture
guide).

## The workspace at a glance

| Crate | Role |
|-------|------|
| `seiva-ports` | Trait-only boundary the application layer codes against (`UnitOfWork`, `IdGenerator`, `EventPublisher`, `Clock`). No deps. |
| `seiva-macros` | The procedural macros: `#[seiva::facade]`, `#[seiva::controller]`. |
| `delonix-seiva` | The framework entrypoint: the facade `contract`, the fitness `lint`, the `scaffold`, the worker runtime. |
| `seiva-http` | HTTP transport (`axum`): RFC 9457 problems, validated bodies, the descriptor-driven permission guard, health, `/openapi.json`. |
| `seiva-grpc` | proto3 + `tonic::Status` generation from a facade descriptor. |
| `seiva-openapi` | OpenAPI 3.1 document generation from facade descriptors. |
| `seiva-cqrs` | Application-layer command/query dispatch with typed middleware. |
| `seiva-messaging` | CloudEvents envelope + broker backends (NATS/AMQP/Kafka). |
| `seiva-auth` | Argon2id password hashing (PHC). |
| `seiva-config` | 12-factor typed configuration + a production-safety guard. |
| `seiva-webhooks` | Standard Webhooks signing/verification. |
| `seiva-server` | The runtime that boots a service: HTTP app + worker host + graceful shutdown. |
| `seiva-cli` | The installable CLI (`seiva`): scaffolds, lints, runs, generates docs. |
| `seiva-testing` | In-memory doubles for the ports (dev-dependency). |
| `examples/notes`, `examples/accounts` | Reference five-layer modules. |

The DORM-coupled crates (`seiva-data-dorm`, `seiva-outbox-relay`,
`seiva-rag-worker`, `seiva-mcp`) live in the tree but are excluded from the
default build; they compile against the private DORM repo in a token-gated CI
job.

## Seiva vs other Rust frameworks

Seiva is not another router — `seiva-http` builds *on* one (axum/tower). It sits
where a batteries-included, opinionated framework sits; its closest neighbour is
Loco.rs. An honest comparison (for the FastAPI/Django angle, see
[docs/competitive-landscape.md](docs/competitive-landscape.md)):

| Dimension | axum / actix-web / Poem | Loco.rs | **Seiva** |
| --- | --- | --- | --- |
| Shape | routing + extractors library | Rails-style MVC monolith | modules that own their data |
| Batteries (scaffold, auth, admin, migrations) | ✖ bring your own | ✅ generators, jobs, mailers | ✅ CLI scaffold, auth, admin, migrations |
| Data layer | your choice (unopinionated) | SeaORM (coupled) | **pluggable by contract** — DORM default, sqlx/SeaORM adapters (ADR 0030) |
| Architecture enforcement | none | convention | **compiler-enforced** 5-layer crate graph + fitness lint |
| Monolith ↔ microservice | manual | monolith | **same code, by composition** (`seiva module split`) |
| One contract → HTTP + gRPC + OpenAPI | manual (Poem derives OpenAPI only) | partial | **one facade → all three**, no reflection |
| Maturity / ecosystem | ✅ large (axum, actix) | 🟡 growing | ✖ nascent |

**Reach for Seiva** when you want Django/Rails completeness in native Rust, with
Clean Architecture you cannot accidentally violate, module-level zero-trust, and
the freedom to run one binary or many from the same code — without being wedded
to a single ORM. **Reach for plain axum/actix** for a small service where a
router plus a handful of crates is enough, or where five crates per module is
more ceremony than the problem needs. Seiva trades a steeper start for structure
that holds up in high-criticality systems.

## Status

The framework is wired end to end and green in CI: the facade/controller/CQRS
core, the three transports (HTTP, gRPC, OpenAPI) generated from one descriptor,
the `seiva-server` runtime, the scaffolder, and two reference modules (`notes`,
`accounts`). Persistence through DORM is wired behind the `dorm` feature and
activates once the private-repo CI token is configured.

The library crates are **published on crates.io** (v0.1.0) under a dual
MIT/Apache licence. This is an early release: the API may still change before
1.0, and the ecosystem is nascent — feedback and contributions are welcome (see
[CONTRIBUTING.md](CONTRIBUTING.md)).

See [`docs/adr/`](docs/adr/) for the design decisions and
[`docs/progress.md`](docs/progress.md) for the roadmap state.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your
option.
