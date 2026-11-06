# The `seiva` CLI

`seiva` is the installable command-line tool (binary `seiva`, crate
`seiva-cli`): it scaffolds projects and modules, runs the dev loop, drives the
data layer, lints the architecture, and prints the generated contracts. It is
Seiva's single CLI for the whole project lifecycle.

This page is a **tour in lifecycle order** — read top to bottom and you walk a
project from nothing to a deployed, split service. Each step shows the shell. For
the data-layer deep dive see [the data layer](../topics/data-layer/index.md); for
settings see [Settings and environment](settings.md).

## Install

The CLI is a global tool — install it with Cargo (until the crates are published,
from the workspace):

```console
$ cargo install --path crates/seiva-cli       # or, once published: cargo install seiva-cli
$ seiva --help
```

> **`dx` users:** to pin `seiva` as a project-local, offline tool instead, use
> `dx tool install --path crates/seiva-cli` and run it via `dx tool run seiva …`.
> `dx` otherwise manages your project's build and dependencies (`dx build`,
> `dx run`, `dx add`), Cargo-compatibly.

## The commands at a glance

```
project  Create a new Seiva project
module   Manage modules within a project (new / split / merge)
ext      Integrate an optional extension (e.g. the admin console)
db       Data-layer commands (DORM modules only)
start    Run the project locally (the dev loop)
openapi  Print the OpenAPI 3.1 document for the project (or the demo facade)
proto    Print the proto3 service document
lint     Run the architecture fitness functions locally
expand   Show what the Seiva macros expand to
doctor   Report the environment: framework version and linked data layer
user     Manage user credential records (Argon2id)
```

## 1. Create a project — `seiva project new <name>`

Scaffolds a Cargo workspace (a **modular monolith**): the composition root
(`apps/<name>-api`), the shared kernel, a `schema/<name>.schema`, and the
cloud-native packaging (`Delonixfile`, `compose.yaml`, `.dockerignore`). It
compiles standalone — the workspace root carries an interim `[patch.crates-io]`
pointing the framework crates at a local checkout until they are published
(override with `SEIVA_FRAMEWORK_DIR`; see [Publishing](../publishing.md)).

```console
$ seiva project new polls-site
Created project `polls-site`. Next: cd polls-site && seiva module new <name>
$ cd polls-site
```

## 2. Check the environment — `seiva doctor`

Reports the framework version and which data layer is linked (in-memory vs DORM).
Run it any time you are unsure what a binary will do.

```console
$ seiva doctor
```

## 3. Add a module — `seiva module new <name> [--data external] [--admin]`

Scaffolds a five-layer module (facade, domain, application, infrastructure,
interface) wired to the real framework transports, and registers it in the
workspace, `seiva.toml` and the schema — and merges its router into the
composition root, so it is served immediately.

```console
$ seiva module new polls
Created module `polls` (facade v1, 5 layers). Registered in the workspace, seiva.toml and schema, and wired into the composition root.
```

`--admin` registers an admin resource (surfaced by the optional
`seiva-console-admin` extension); `--data external` opts the module out of DORM.

## 4. Run it — `seiva start`

Runs the project's `apps/<name>-api` server in development — Seiva's dev-server
command. Add `--watch` for hot-reload (needs `cargo-watch`).

```console
$ seiva start                 # or, directly: dx run -p polls-site-api / cargo run -p polls-site-api
```

The service serves `/health`, `/openapi.json`, and the interactive docs:
`/docs` (Swagger UI), `/redoc` (ReDoc) and `/scalar` (Scalar) — all three
generated from your facades with no annotations. Open
<http://localhost:8080/docs> to try calls against the running service.

## 5. Inspect the contract — `seiva openapi` / `seiva proto`

```console
$ seiva openapi               # the PROJECT's OpenAPI 3.1 (build+run, emit, exit); --compact for one line
$ seiva openapi | grep -o '"/v1/polls"'
"/v1/polls"
$ seiva proto                 # the project's proto3 service document
```

Inside a project these document **your** modules via link-time facade discovery
(no central list — OWASP API9); run outside a project and they document the
framework's built-in demonstration facade instead. The bodies are *faithful* —
the DTO directly, with its real fields (see
[API documents](../topics/api-documents.md)).

## 6. The data layer — `seiva db …` (DORM modules)

```console
$ seiva db init                       # PostgreSQL (default); or --backend mysql | sqlite
$ export DELONIX_ORM_DSN="host=localhost user=me dbname=polls sslmode=disable"
$ seiva db dev initial                # makemigrations + migrate (author + apply the next migration)
$ seiva db migrate                    # migrate deploy (apply the recorded chain, in dependency order)
$ seiva db pull                       # brownfield: introspect an existing DB into a schema
$ seiva db studio                     # browse/edit data in the browser (Prisma Studio-style)
```

These delegate to the DORM CLI (`dorm`), found on `PATH` or via `SEIVA_DORM_BIN`.
See [Migrations](../topics/data-layer/migrations.md) and
[Making queries](../topics/data-layer/queries.md).

## 7. Split a module into a microservice — `seiva module split` / `merge`

Promote a module to its own service without changing its code: `split` scaffolds
a standalone `apps/<name>-service` binary that mounts only that module, registers
it in the workspace, and sets `mode = "service"` in the module's `module.toml`.
The module keeps running inside the monolith too — this adds a deployment shape.

```console
$ seiva module split polls
Split module `polls` into a service. Created apps/polls-service (mounts only `polls`), registered it in the workspace and set mode = "service".
$ cargo run -p polls-service          # or dx run -p polls-service — serves ONLY polls
$ seiva module merge polls            # fold it back: removes the service, resets mode = "library"
```

See the tutorial's
[monolith → microservice](../getting-started/shop/part-9-monolith-to-microservices.md).

## 8. Add an optional extension — `seiva ext add <name>`

Integrates an optional extension into the composition root (ADR 0010): adds the
dependency and inserts the mount at the managed anchor — explicit, idempotent,
and it executes no install script.

```console
$ seiva ext add seiva-console-admin   # mount the admin console at /admin
```

## 9. Keep the architecture honest — `seiva lint` / `seiva expand`

```console
$ seiva lint                  # the dependency rule + the anti-pattern catalogue
$ seiva expand                # show what #[facade]/#[controller]/#[derive(WireSchema)] expand to
```

`lint` is the fitness-function pass (see [Testing](../topics/testing.md));
`expand` is the "no hidden magic" inspector.

## 10. Users — `seiva user create --username <NAME>`

```console
$ seiva user create --username alice  # an Argon2id credential record; the password is read from STDIN
```

## Related

- [Getting started](../getting-started/index.md) · [Quickstart](../quickstart.md)
- [Settings and environment](settings.md)
- [The crates](crates/index.md)

---

*Part of the [Seiva documentation](../index.md).*
