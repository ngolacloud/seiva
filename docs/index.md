# Seiva documentation

<div class="seiva-hero" markdown>
# Delonix Seiva
A Rust backend framework for robust, high-criticality systems — **ergonomic
request handling** and **batteries included**, delivered as a single, compiled,
no-GC binary. Your handlers are plain `async fn`; a module owns its data and
exposes **one machine-readable contract**, and from it Seiva *derives* the HTTP
routes, the interactive API docs, the permission checks and the gRPC service —
so you write behaviour, not boilerplate.
</div>

> **New here?** Read [Seiva at a glance](getting-started/seiva-at-a-glance.md),
> then the [Quick install guide](getting-started/quick-install.md). Coming from
> a dynamic framework? Start with [your first day](first-day.md). Want to build something end
> to end? The [tutorial](getting-started/shop/part-1-the-modular-monolith.md)
> builds a polls app as a monolith and then splits a module into a microservice.

## Why Seiva

<div class="grid cards" markdown>

-   :material-flash: **Async by default**

    Handlers are native `async fn` on a multi-threaded Tokio runtime — a
    familiar async mental model, with no GC and no event-loop surprises.

-   :material-file-document-check: **Docs you never write**

    Every service serves `/openapi.json` and interactive **Swagger UI** (`/docs`),
    **ReDoc** (`/redoc`) and **Scalar** (`/scalar`) out of the box, generated from
    the contract. No annotations, no spec to maintain.

-   :material-shield-lock: **Secure and typed end to end**

    Deny-by-default permissions per method, RFC 9457 problems, and DTOs whose real
    fields are published to OpenAPI — the compiler keeps the wire honest.

-   :material-arrange-send-backward: **Monolith *or* microservices**

    Start as one modular binary; promote any module to its own service with
    `seiva module split` when you need to — the same code, a deployment knob.

</div>

## How the documentation is organized

This documentation is organized in four modes — the
[Diátaxis](https://diataxis.fr) model — because it keeps *learning* separate from
*looking up*. (See [the structure map](STRUCTURE.md) for how the sections fit
together.)

<div class="grid cards" markdown>

-   :material-school: **[Tutorials](getting-started/index.md)**

    ---

    Learning-oriented, hand-held lessons that take you from an empty checkout to
    a running, authenticated, packaged service.

    [:octicons-arrow-right-24: Start the tutorial](getting-started/shop/part-1-the-modular-monolith.md)

-   :material-book-open-variant: **[Topic guides](topics/index.md)**

    ---

    Understanding-oriented discussions of each layer: the data layer, the
    transport, validation, identity, security, observability, events.

    [:octicons-arrow-right-24: Using Seiva](topics/index.md)

-   :material-file-document-multiple: **[Reference](reference/index.md)**

    ---

    Information-oriented, exhaustive descriptions of the CLI, the settings, the
    contract descriptor and the crates.

    [:octicons-arrow-right-24: Look it up](reference/index.md)

-   :material-wrench: **[How-to guides](howto/index.md)**

    ---

    Task-oriented recipes for specific problems: add a module, switch to DORM,
    deploy distroless.

    [:octicons-arrow-right-24: Recipes](howto/index.md)

</div>

### Highlights

<div class="grid cards" markdown>

-   :material-sitemap: **[Architecture](architecture.md)**

    ---

    The layers, the facade contract, and the transports generated from it.

-   :material-speedometer: **[Benchmarks](benchmarks.md)**

    ---

    Measured throughput and latency on the production path, against interpreted
    stacks.

</div>

## Getting started

- [Seiva at a glance](getting-started/seiva-at-a-glance.md)
- [Quick install guide](getting-started/quick-install.md) · the fuller
  [quickstart](quickstart.md)
- The tutorial: [1 · Your first module](getting-started/shop/part-1-the-modular-monolith.md)
  → [2 · Validation](getting-started/shop/part-2-modules-own-their-data.md)
  → [3 · Data layer](getting-started/shop/part-4-the-data-layer.md)
  → [4 · Authentication](getting-started/shop/part-5-authorization.md)
  → [5 · Testing](getting-started/shop/part-8-testing.md)
  → [6 · Packaging & deploy](getting-started/shop/part-9-monolith-to-microservices.md)
  → [7 · Monolith to microservice](getting-started/shop/part-9-monolith-to-microservices.md)
- [Coming from a dynamic framework](first-day.md)

## The layers (topic guides)

| Concern | Seiva topic |
| --- | --- |
| The model layer | [The data layer (DORM)](topics/data-layer/index.md) |
| The view layer | [The transport layer](topics/transport/index.md) |
| Forms | [Validation and DTOs](topics/validation.md) |
| The template layer | [API documents (OpenAPI / docs UI / proto)](topics/api-documents.md) |
| Auth system | [Authentication](topics/authentication.md) · [Authorization](topics/authorization.md) |
| Security | [Security](topics/security.md) |
| Logging | [Observability](topics/observability.md) |
| Signals / tasks | [Messaging & events](topics/messaging-and-events.md) · [Background workers](topics/background-workers.md) |
| Testing | [Testing](topics/testing.md) |
| Settings | [Configuration](topics/configuration.md) |
| Performance | [Performance & cloud-native](topics/performance.md) |

## Reference

- [The `seiva` CLI](reference/cli.md)
- [Settings and environment](reference/settings.md)
- [The contract descriptor](reference/contract-descriptor.md) ·
  [Wire schema](reference/wire-schema.md)
- [The crates](reference/crates/index.md)

## The Seiva project

- [Architecture](architecture.md) and the [design internals](internals/index.md)
- [Architecture Decision Records](internals/adrs.md) (the [`adr/`](adr/0001-seiva-foundations.md) directory)
- [FAQ](faq/index.md) · [Glossary](glossary.md)
- [Competitive landscape](competitive-landscape.md) and the [roadmap](roadmap.md)
- [Release notes](releases/index.md) and the [engineering log](progress.md)
- [Publishing status](publishing.md) · [Contributing](internals/contributing.md)

---

*This documentation follows the [Diátaxis](https://diataxis.fr) model.
Pages marked “planned” carry the intended outline; the framework’s capabilities
are tracked honestly in [progress.md](progress.md) and the ADRs.*
