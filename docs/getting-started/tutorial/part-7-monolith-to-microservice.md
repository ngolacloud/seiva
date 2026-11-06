# Part 7: From monolith to a microservice

Parts 1–6 built polls as a **modular monolith** — one binary, `apps/polls-site-api`,
with the `polls` module bounded inside it. That is the right default: one thing to
build, deploy and reason about. But a module boundary in Seiva is a *real* seam,
and when one module needs to scale, deploy or fail on its own, you can promote it
to a **microservice** — without touching the module's code.

This is the payoff of the five-layer boundary you have been respecting all along:
the `polls` module already depends on nothing it shouldn't, so standing it up on
its own is a scaffolding step, not a rewrite.

## Split the module

```console
$ seiva module split polls
Split module `polls` into a service. Created apps/polls-service (mounts only `polls`), registered it in the workspace and set mode = "service".
Run the microservice: `cargo run -p polls-service` (or `dx run -p polls-service`). The monolith still serves `polls` too; `seiva module merge polls` undoes this.
```

`seiva module split` does three things, all of them legible edits you can read in
the diff:

1. **Scaffolds `apps/polls-service/`** — a second composition root (its own `main`)
   that mounts **only** the `polls` module, over the same runtime serve loop as the
   monolith. It gets the framework routes for free: `/health`, `/openapi.json`, and
   the interactive docs (`/docs`, `/redoc`, `/scalar`).
2. **Registers it** as a workspace member.
3. **Records `mode = "service"`** in `modules/polls/module.toml`.

It moves **no code**. The five `modules/polls/` crates are untouched, and the
monolith still serves `polls` as before — you now simply have a second way to run
the same module.

## Run the microservice

=== "dx"

    ```console
    $ dx run -p polls-service
    ```

=== "cargo"

    ```console
    $ cargo run -p polls-service
    ```

It is a complete Seiva service that happens to host one module. Point it at a
port and it serves the `polls` routes and its own OpenAPI:

```console
$ SEIVA_HTTP_ADDR=127.0.0.1:8081 cargo run -p polls-service &
$ curl -s localhost:8081/openapi.json | grep -o '"/v1/polls"'
"/v1/polls"
$ curl -s -o /dev/null -w '%{http_code}\n' localhost:8081/docs    # Swagger UI
200
```

The service's contract is also printable offline, exactly like the monolith's:

```console
$ SEIVA_EMIT=openapi cargo run -p polls-service    # just the polls facade
```

## What runs where

| | Monolith `apps/polls-site-api` | Microservice `apps/polls-service` |
| --- | --- | --- |
| Modules served | every module in the project | only `polls` |
| Deploy unit | one binary/image | its own binary/image (its own `Delonixfile`) |
| Scales | as a whole | independently |
| Module code | `modules/polls/*` | the **same** `modules/polls/*` |

Because both roots link the identical module crates, there is no second
implementation to keep in sync — the contract, the use cases and the tests are
shared. You choose the deployment shape; the module does not change.

## Going back

Changed your mind, or the traffic settled? Fold it back into the monolith:

```console
$ seiva module merge polls
Merged `polls` back into the monolith: removed apps/polls-service and reset mode = "library". The module still serves inside apps/<project>-api.
```

`merge` removes `apps/polls-service`, drops its workspace member and resets the
mode. The `polls` module itself is, again, untouched.

## gRPC

A split service is also where gRPC earns its place: the same facade already
generates a proto3 service document (`seiva proto`), so a `polls` microservice can
expose gRPC alongside HTTP for service-to-service calls. See
[API documents](../../topics/api-documents.md#proto3-seiva-proto).

## The end — and the beginning

You built a complete polls app (data, validation, auth, results, tested,
packaged), then deployed it two ways from one codebase. Where next:

- The [topic guides](../../topics/index.md) for depth on each layer.
- [Add a module](../../howto/add-a-module.md) and
  [switch to DORM](../../howto/switch-to-dorm.md) how-tos.
- The [CLI reference](../../reference/cli.md) for the full command tour.

---

*Part of the [Seiva documentation](../../index.md).*
