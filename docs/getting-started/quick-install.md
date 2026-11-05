# Quick install guide

## Prerequisites

A recent stable Rust toolchain (`rustup`). A database is optional — a scaffolded
project runs in-memory until you enable the data layer.

## Install the CLI

Until the crates are published to crates.io, install `seiva` from the workspace:

```console
$ cargo install --path crates/seiva-cli
$ seiva --version
```

(Once published: `cargo install seiva-cli`.)

## Create and run a project

```console
$ seiva project new demo
$ cd demo
$ seiva module new notes       # a module, auto-wired into the composition root
$ seiva start                  # run it (add --watch for hot-reload)
```

The service serves `/health`, `/openapi.json`, `/docs` and the module's
`POST /v1/notes`. (`/admin` is an optional extension — see
[API documents](../topics/api-documents.md); the base returns `404` for it.)
Check `seiva doctor` for the framework version and which data layer is linked.

## Verify

```console
$ seiva openapi | grep '/v1/notes'      # the project's own contract
```

## Next

Follow the [tutorial](tutorial/part-1-your-first-module.md), or if you come from
FastAPI start with [your first day](../first-day.md). The fuller walk-through is
the [quickstart](../quickstart.md).

---

*Part of the [Seiva documentation](../index.md).*
