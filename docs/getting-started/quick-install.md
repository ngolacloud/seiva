# Quick install guide

## Prerequisites

A recent stable Rust toolchain. You can manage it with `rustup` and Cargo, or
with [**`dx`**](https://ngolacloud.github.io/dx/) — Delonix's UV-style Rust
package manager, fully Cargo-compatible (it drives `cargo`/`rustc` under the
hood). This guide shows both; pick one. A database is optional — a scaffolded
project runs in-memory until you enable the data layer.

## Install the CLI

The `seiva` CLI is a global tool, so install it with Cargo (until the crates are
published, from the workspace):

```console
$ cargo install --path crates/seiva-cli        # or, once published: cargo install seiva-cli
$ seiva --version
```

> **Using `dx`?** `dx` is the *project* package manager (it drives your build and
> dependencies — see below). To pin `seiva` as a project-local, offline tool
> instead of a global install, use `dx tool install --path crates/seiva-cli` and
> run it with `dx tool run seiva …`.

## Create and run a project

```console
$ seiva project new demo
$ cd demo
$ seiva module new notes       # a module, auto-wired into the composition root
$ seiva start                  # run it (add --watch for hot-reload)
```

The service serves `/health`, `/openapi.json`, the interactive docs
(`/docs` = Swagger UI, `/redoc` = ReDoc, `/scalar` = Scalar) and the module's
`POST /v1/notes`. (`/admin` is an optional extension — see
[API documents](../topics/api-documents.md); the base returns `404` for it.)
Check `seiva doctor` for the framework version and which data layer is linked.

## Verify

```console
$ seiva openapi | grep '/v1/notes'      # the project's own contract
```

Then open <http://localhost:8080/docs> for the Swagger UI, where you can try the
`POST /v1/notes` call against the running service.

## Next

Follow the [tutorial](tutorial/part-1-your-first-module.md), or if you come from
FastAPI start with [your first day](../first-day.md). The fuller walk-through is
the [quickstart](../quickstart.md).

---

*Part of the [Seiva documentation](../index.md).*
