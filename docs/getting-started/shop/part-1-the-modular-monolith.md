# Part 1: A modular monolith

The shop starts life as **one binary** — a modular monolith. You get the
simplicity of a single deploy, and because each concern is a separate module with
its own contract, you keep the option to split it later (Part 7) without a
rewrite.

## Scaffold the project

```console
$ seiva project new shop
$ cd shop
```

## Add the five modules

```console
$ seiva module new catalog
$ seiva module new inventory
$ seiva module new orders
$ seiva module new payments
$ seiva module new notifications
```

Each `seiva module new` creates a five-layer module under `modules/<name>/` and
wires it into the composition root. After the five, the workspace looks like this:

```text
shop/
├── Cargo.toml
├── seiva.toml
├── apps/
│   └── shop-api/                # the composition root — the one binary
├── shared/
│   └── kernel/                  # ids, errors, time
├── modules/
│   ├── catalog/                 # products
│   ├── inventory/               # stock levels
│   ├── orders/                  # orders
│   ├── payments/                # payments
│   └── notifications/           # sent messages
├── schema/
│   └── shop.schema              # the DORM schema (Part 4)
├── Delonixfile                  # distroless image
└── compose.yaml                 # local run
```

Every module is five crates (`facade`, `domain`, `application`, `infrastructure`,
`interface`) and the compiler enforces the dependency rule between them — see
[Architecture](../../architecture.md). The modules are **independent**: none
depends on another module's crates. That independence is what makes Part 7's split
a composition change rather than a refactor.

## Run it

```console
$ seiva start
```

The one binary serves `/health`, `/openapi.json`, the interactive docs, and every
module's routes under `/v1/<module>`. The whole shop is one OpenAPI document, one
process, one deploy — for now.

```console
$ seiva openapi | grep -oE '"/v1/(catalog|inventory|orders|payments|notifications)[^"]*"' | sort -u
```

## Next

[Part 2: Modules own their data](part-2-modules-own-their-data.md) — the five
contracts and the permissions they declare.

---

*Part of the [Seiva documentation](../../index.md).*
