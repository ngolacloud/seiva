# Seiva at a glance

Seiva lets you write a backend as a **compiled Rust binary**: fast request
handling with one source of truth and generated docs, plus the *batteries* you
expect — a data layer, an auth system, security by default, and a project
scaffolder.

## The one big idea

**A module owns its data and exposes a single, machine-readable contract. Every
transport is derived from that contract — never hand-written.**

You write a *facade* — a versioned trait annotated with `#[seiva::facade]`:

```rust
#[facade(version = "v1")]
pub trait Notes {
    #[requires("notes.create")]
    async fn create(&self, cmd: CreateNote) -> Result<NoteDto, String>;
}
```

From that one declaration Seiva derives:

- the **HTTP route** (`POST /v1/notes`) via `#[seiva::controller]`;
- the **OpenAPI 3.1 document** — with the *real* request/response shapes, because
  the DTOs derive [`WireSchema`](../reference/wire-schema.md) (no `cmd` envelope,
  no opaque placeholders);
- the **permission check** (`notes.create`), enforced before the handler runs;
- a **proto3** service document (`seiva proto`).

No reflection: it is all known at compile time, and every macro has a documented
hand-written equivalent ("no hidden magic").

## A module is five crates

The compiler enforces the architecture — you cannot violate the dependency rule
and still build (see [Architecture](../architecture.md)):

```
interface ─▶ application ─▶ domain
   │             │
   └─▶ facade ◀──┘        infrastructure ─▶ application
```

- **facade** — the versioned contract + wire DTOs (`serde`, `Validate`, `WireSchema`).
- **domain** — entities and rules; depends on nothing but the shared kernel.
- **application** — the use case over trait-only *ports*; transport- and
  data-layer-free, so it unit-tests with no server or database.
- **infrastructure** — adapters (in-memory by default; DORM-backed via a feature).
- **interface** — the `#[seiva::controller]` and the OpenAPI generation.

## What you get

- **Language / runtime** — Rust, no GC, a single compiled binary.
- **Auto OpenAPI + docs UI** — faithful, generated from the contract.
- **Type-driven validation** — `serde` + `Validate`, RFC 9457 problems.
- **ORM / data layer** — DORM, optional and feature-gated.
- **Project scaffolding** — `seiva project new` / `seiva module new`.
- **Built-in auth** — verified JWT/OIDC via `seiva-identity`, deny-by-default.
- **Cloud-native build** — distroless, non-root, `/health`.
- **Migrations and an admin** — in progress (see [progress](../progress.md)).

Seiva's bet: **an ergonomic, batteries-included developer experience, without a
GC, deployed as one distroless binary.** For an honest, measured scorecard
against other stacks, see the [competitive landscape](../competitive-landscape.md).

## What exists today

Seiva is at founding stage. Working today: the facade/controller/OpenAPI pipeline
(with faithful `WireSchema` schemas), deny-by-default OIDC/JWT authentication,
object-level authorization, the five-layer scaffolder, a standalone-compiling
generated project, and distroless packaging. In progress: the DORM data-layer
cutover, migrations, and an admin. The [engineering log](../progress.md) and the
[ADRs](../internals/adrs.md) track it honestly.

---

*Part of the [Seiva documentation](../index.md). Next:
[Quick install](quick-install.md).*
