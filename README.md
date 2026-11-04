# Delonix Seiva

A Rust backend framework for robust, high-criticality systems — backend only,
with the ease of FastAPI and NestJS, a production- and cluster-ready server, and
[DORM](https://github.com/ngolacloud/dorm) as the default (optional) data layer.

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

## DORM is the default, and optional

The DORM data layer is wired in by default, but as a cargo feature. A project (or
a single module) whose data lives elsewhere switches it off and pulls in no ORM
at all, owning its own connections with the libraries it chooses.

The framework **core never depends on DORM**: the ORM enters only through the
`seiva-data-dorm` gateway behind the `dorm` feature. The dependency is one-way —
Seiva depends on DORM, DORM never on Seiva. Everything in this workspace builds
and tests **without** the private DORM repository; the DORM-coupled crates are a
separate, token-gated build (see the architecture guide).

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

## Status

The framework is wired end to end and green in CI: the facade/controller/CQRS
core, the three transports (HTTP, gRPC, OpenAPI) generated from one descriptor,
the `seiva-server` runtime, the scaffolder, and two reference modules (`notes`,
`accounts`). Persistence through DORM is wired behind the `dorm` feature and
activates once the private-repo CI token is configured.

See [`docs/adr/`](docs/adr/) for the design decisions and
[`docs/progress.md`](docs/progress.md) for the roadmap state.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your
option.
