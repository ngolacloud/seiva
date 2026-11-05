# Seiva documentation

<div class="seiva-hero" markdown>
# Delonix Seiva
Delonix **Seiva** is a Rust backend framework for robust, high-criticality
systems — the ergonomics of FastAPI and the batteries of Django, delivered as a
single, compiled, no-GC binary. A module owns its data and exposes **one
machine-readable contract**; the HTTP routes, the OpenAPI document, the
permission checks and the gRPC service are all *derived* from that contract.
</div>

> **New here?** Read [Seiva at a glance](getting-started/seiva-at-a-glance.md),
> then the [Quick install guide](getting-started/quick-install.md). Coming from
> FastAPI? Start with [your first day](first-day.md).

## How the documentation is organized

This documentation follows the same four-mode structure as the Django docs — the
[Diátaxis](https://diataxis.fr) model — because it keeps *learning* separate from
*looking up*. (See [the structure map](STRUCTURE.md) for how each Django section
maps to Seiva.)

<div class="grid cards" markdown>

-   :material-school: **[Tutorials](getting-started/index.md)**

    ---

    Learning-oriented, hand-held lessons that take you from an empty checkout to
    a running, authenticated, packaged service.

    [:octicons-arrow-right-24: Start the tutorial](getting-started/tutorial/part-1-your-first-module.md)

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

    Seiva vs FastAPI vs Django — measured throughput and latency on the
    production path.

</div>

## Getting started

- [Seiva at a glance](getting-started/seiva-at-a-glance.md)
- [Quick install guide](getting-started/quick-install.md) · the fuller
  [quickstart](quickstart.md)
- The tutorial: [1 · Your first module](getting-started/tutorial/part-1-your-first-module.md)
  → [2 · Validation](getting-started/tutorial/part-2-validation-and-dtos.md)
  → [3 · Data layer](getting-started/tutorial/part-3-the-data-layer.md)
  → [4 · Authentication](getting-started/tutorial/part-4-authentication.md)
  → [5 · Testing](getting-started/tutorial/part-5-testing.md)
  → [6 · Packaging & deploy](getting-started/tutorial/part-6-packaging-and-deploy.md)
- [Coming from FastAPI](first-day.md)

## The layers (topic guides)

| Django analogue | Seiva topic |
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

*This structure mirrors the [Django 6.1 documentation](https://docs.djangoproject.com/en/6.1/).
Pages marked “planned” carry the intended outline; the framework’s capabilities
are tracked honestly in [progress.md](progress.md) and the ADRs.*
