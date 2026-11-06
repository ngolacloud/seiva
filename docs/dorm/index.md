# DORM — the Delonix ORM

**DORM** is Seiva's default data layer: a native-Rust, compile-time-typed ORM.
It is its own product — its own repository, crates on crates.io, its own CLI and
book — that Seiva *consumes*. This section is the full picture of DORM as a Seiva
developer meets it: the schema language, the typed query API, migrations,
transactions, backends, the tooling, and exactly how Seiva wires it in.

> If you only want the framework-level view — the ports your application codes
> against and how data stays out of your use cases — read the
> [data-layer topic guide](../topics/data-layer/index.md) first. This section
> goes one level deeper, into DORM itself.

## What DORM is

You describe your data once, in a declarative `.schema` file. DORM compiles that
into Rust model structs with `#[derive(Model)]`, and from the *same* metadata it
generates the SQL schema, the migrations, and a lazy, chainable, typed query
builder (`QuerySet`). The schema is the single source of truth; the model code is
generated and not edited by hand.

```rust
// Typed, lazy, reusable — nothing runs until you await a terminal method.
let newest = Author::objects()
    .filter(author::name.icontains("ada"))
    .order_by([author::name.asc()])
    .limit(10)
    .all()
    .await?;
```

Two things make DORM distinctive, and both are structural rather than stylistic:

- **Safe by construction.** Every *value* travels as a bound parameter (`$1`,
  `$2`, …), and every *identifier* is a `&'static str` the derive macro emitted
  into the binary. There is no code path that formats a value into SQL and no way
  for a caller to name a table or column the compiler did not already know. SQL
  injection is not a bug you can write.
- **One compiler, two front ends.** The typed API (`author::name.eq("ada")`) and
  a string-keyed dynamic API (`.filter_str("name__icontains", "ada")`) build the
  *same* query plan. The dynamic one is safe to expose to a REPL or an admin
  console because it can only resolve names the macro registered.

## Why a native Rust ORM

DORM is **native Rust end to end**: no garbage collector, no foreign-function
boundary between a query engine and your code, compile-time type safety all the
way to the binary, and a single-binary deploy. The metadata the queries read is
the metadata the migration generator reads, so the schema your code assumes and
the schema your database has cannot silently diverge.

DORM is also deliberately *independent*. Its crates carry no application or
business logic; any Rust program can depend on `dorm-core` alone. That
independence is what lets Seiva treat DORM as a replaceable data-source layer
rather than a framework it is fused to.

## Where DORM sits under Seiva

Seiva's core **never depends on DORM**. The coupling lives in exactly one place —
the `seiva-data-dorm` gateway, compiled only when you turn on the `dorm` feature.
A module's application layer codes against trait-only *ports* (`UnitOfWork`,
`EventPublisher`, `Clock`, `IdGenerator`, and its own repository trait); the
gateway is the adapter that satisfies those ports with DORM.

```
application ──(RecordRepository / UnitOfWork ports)──▶ infrastructure
                                                         ├─ in-memory adapters   (default, DORM-free)
                                                         └─ DORM-backed adapters  (feature = "dorm")
```

The public Seiva workspace is **DORM-free by default**: a project whose every
repository uses an external data source pulls in no ORM at all. See
[Using DORM from Seiva](using-from-seiva.md) for the gateway, the feature flag,
and the `seiva db *` commands that delegate to DORM.

## The crates

| Crate | What it is |
|---|---|
| `dorm-core` | The runtime: `QuerySet`/`Manager`, the query compiler, `Value`, the model registry, the connection pool and transactions |
| `dorm-derive` | `#[derive(Model)]` — turns a struct into a model and emits its typed field handles and `ModelMeta` |
| `dorm-schema` | The `.schema` parser and code generator (declarative schema → Rust models) |
| `dorm-migrations` | DDL generation, the schema autodetector, and the versioned migrator |
| `dorm-sqlite` | The SQLite executor |
| `dorm-shell` | The `dorm-shell` REPL — enumerates the linked models and runs string-typed queries live |
| `dorm-studio` | DORM Studio — the web UI over your live database |
| `dorm-cli` | The installable project manager (`cargo install dorm-cli`, the `dorm` binary) |

Optional transactional-support crates — `dorm-outbox`, `dorm-jobs`,
`dorm-eventstore` — implement ports the layer above owns; `dorm-core` never
depends on them. That boundary is fixed in DORM's own ADR 0054.

## In this section

- **[Schema and models](schema-and-models.md)** — the `.schema` DSL
  (`datasource`, `namespace`, `model`, fields, attributes, enums, relations) and
  the model code it generates.
- **[Making queries](queries.md)** — the typed `QuerySet`: CRUD, filters,
  ordering, pagination, aggregates, projections.
- **[Relations](relations.md)** — foreign keys, filtering across them, and
  joining the related rows in one round-trip.
- **[Migrations](migrations.md)** — generating and applying schema, the ledger,
  and how the workflow surfaces through Seiva.
- **[Transactions](transactions.md)** — `atomic`, savepoints, read-only mode, and
  how Seiva's `UnitOfWork` port maps onto DORM.
- **[Raw SQL](raw-sql.md)** — the parameterized escape hatch and its guardrails.
- **[Backends](backends.md)** — PostgreSQL, MySQL, SQLite, CockroachDB, and the
  DSN-agnostic data path, TLS and pooling.
- **[The CLI and Studio](cli-and-studio.md)** — the `dorm` command surface, the
  REPL, and the Studio GUI.
- **[Using DORM from Seiva](using-from-seiva.md)** — the gateway, the outbox
  relay, the `seiva db *` commands, and the `dorm` feature.

---

*Part of the [Seiva documentation](../index.md).*
