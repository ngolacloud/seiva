# The `seiva` CLI

`seiva` is the installable command-line tool (binary `seiva`, crate
`seiva-cli`): it scaffolds projects and modules, runs the dev loop, lints the
architecture, and prints the generated contracts. It is Seiva's answer to both
`django-admin`/`manage.py` and `fastapi-cli`.

Install it from the workspace (until the crates are on crates.io):

```console
$ cargo install --path crates/seiva-cli
$ seiva --help
```

## Commands

```
project  Create a new Seiva project
module   Manage modules within a project
db       Data-layer commands (DORM modules only)
lint     Run the architecture fitness functions locally
expand   Show what the Seiva macros expand to
start    Run the project locally (dev loop)
doctor   Report the environment: framework version and which data layer is linked
openapi  Print the OpenAPI 3.1 document for the built-in demonstration facade(s)
proto    Print the proto3 service document for the built-in demonstration facade(s)
user     Manage user credential records
```

### `seiva project new <name>`

Scaffolds a new project: the workspace, the composition root (`apps/<name>-api`),
the shared kernel, a schema file, and the cloud-native packaging (`Delonixfile`,
`compose.yaml`, `.dockerignore`). The generated project compiles **standalone** —
the workspace root carries an interim `[patch.crates-io]` pointing the framework
crates at a local checkout until they are published (override with
`SEIVA_FRAMEWORK_DIR`; see [Settings](settings.md) and [Publishing](../publishing.md)).

### `seiva module new <name> [--data external]`

Scaffolds a five-layer module (facade, domain, application, infrastructure,
interface) wired to the real framework transports, and registers it in the
workspace, `seiva.toml` and the schema. `split` / `merge` move a module to and
from its own `service` schema and history.

### `seiva db dev` / `seiva db migrate`

Data-layer commands for DORM modules: `dev` generates models and creates/applies a
migration for one module locally; `migrate` applies every DORM module's migrations
in dependency order. (Requires the `dorm` feature.)

### `seiva start`

Runs the project's `apps/<name>-api` server in development — the dev loop
(Seiva's `fastapi dev` / `runserver`).

### `seiva lint`

Runs the architecture fitness functions locally — the dependency rule and the
anti-pattern catalogue (see [Testing](../topics/testing.md)).

### `seiva expand`

Shows what the Seiva macros expand to — the "no hidden magic" inspector for
`#[facade]`, `#[controller]` and `#[derive(WireSchema)]`.

### `seiva doctor`

Reports the environment: the framework version and which data layer is linked
(in-memory vs DORM).

### `seiva openapi [--pretty|--compact]` / `seiva proto`

Print the generated OpenAPI 3.1 / proto3 documents for the built-in demonstration
facade. The OpenAPI body and schemas are **faithful** — the DTO directly, with its
real fields (see [API documents](../topics/api-documents.md)).

> **Limitation (this first cut):** `openapi`/`proto` document only the framework's
> built-in demonstration facade (`NotesFacade`); runtime discovery of a project's
> own facades is a later phase. The runtime service's `/openapi.json` *does* list
> every mounted facade via link-time discovery.

### `seiva user create --username <NAME>`

Manage user credential records (Argon2id), the account primitives out of the
data-layer CLI.

## Related

- [Getting started](../getting-started/index.md) · [Quickstart](../quickstart.md)
- [Settings and environment](settings.md)

---

*Part of the [Seiva documentation](../index.md).*
