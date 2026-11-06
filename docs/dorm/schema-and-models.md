# Schema and models

DORM is **schema-first**. You describe your data in a declarative `.schema` file;
`dorm generate` compiles that into Rust model structs with `#[derive(Model)]`.
The schema is the source of truth, and the generated model code is not edited by
hand.

> In a Seiva project the schema lives at `schema/<project>.schema`, and the
> generated DORM data layer lives under `data/`. `seiva db dev` keeps the two in
> step for you — see [Migrations](migrations.md) and
> [Using DORM from Seiva](using-from-seiva.md). This page explains the language
> underneath that workflow; you can also author models as plain Rust structs, the
> target the schema compiles to.

## A complete schema

```
datasource db {
  provider = "postgresql"
  url      = env("DELONIX_ORM_DSN")
}

namespace catalog {
  model Author {
    id        Int      @id @default(autoincrement())
    name      String   @unique @db.VarChar(120)
    createdAt DateTime @default(now())
    posts     Post[]
  }

  model Post {
    id        Int      @id @default(autoincrement())
    author    Author   @relation(onDelete: Cascade)
    title     String   @index @db.VarChar(200)
    createdAt DateTime @default(now())
  }
}
```

`dorm generate` turns each `model` into a `#[derive(Model)]` struct. `Post.author`
becomes a `ForeignKey<Author>`, its reverse `related_name` read off
`Author.posts Post[]`; `Author.posts` itself produces no column (it only names
the back-relation).

## The blocks

| Block | Meaning |
|---|---|
| `datasource <name> { provider = "…"  url = env("VAR") \| "…" }` | The backend and connection. `provider` selects the dialect (PostgreSQL is first-class; see [backends](backends.md)); `url` is read by the runtime, not baked into the binary. |
| `namespace <name> { … }` | A module boundary (see below). |
| `model <Name> { … }` | One table / one Rust struct. |
| `enum <Name> { v1 v2 … }` | A named set of string variants, compiled to a native database enum. |
| `generator … { }` | Accepted and ignored, so a schema written for a Prisma-shaped toolchain pastes in unchanged. |

## Namespaces — the module boundary

A `namespace` block groups the models that belong to one in-process Seiva module.
DORM supports namespaces natively and **forbids a relation that crosses a
namespace**: `dorm validate` rejects a foreign key whose target lives in another
namespace. A module's data therefore cannot entangle with another module's;
only identifiers cross the boundary, the same rule Seiva's facade layer enforces
on the API side.

`seiva module new <m>` appends a `namespace <m>` block with a starter model.
A schema with no `namespace` blocks compiles exactly as before — namespaces are
additive.

## Fields and types

A field is `name Type [@attr …]`. The type determines the column type; a trailing
`?` makes it nullable (`Option<T>` in Rust).

| Schema | Rust | SQL (PostgreSQL) |
|---|---|---|
| `Int` | `i32` | `INTEGER` — `@id @default(autoincrement())` promotes it to `i64` / `BIGSERIAL` |
| `BigInt` | `i64` | `BIGINT` |
| `String` | `String` | `TEXT` — `@db.VarChar(n)` sets a max length |
| `Boolean` | `bool` | `BOOLEAN` |
| `DateTime` | `DateTime<Utc>` | `TIMESTAMPTZ` |
| `Decimal` | `rust_decimal::Decimal` | `NUMERIC` |
| `Json` | `serde_json::Value` | `JSONB` |
| `Uuid` | `uuid::Uuid` | `UUID` |
| `Inet` / `Cidr` / `MacAddr` | `IpAddr` / `IpCidr` / `MacAddress` | `INET` / `CIDR` / `MACADDR` — PostgreSQL only |
| `<Model>` | `ForeignKey<Model>` | a to-one relation; needs `@relation` |
| `<Model>[]` | *(no column)* | a to-many back-relation; names the `related_name` |
| `<Enum>` | `String` | a native database enum (see below) |

The set is deliberately **closed**. An open set would leave the migration
generator unable to name a column's type and would let the dynamic query API ask
for a column the compiler cannot describe.

## Field attributes

| Attribute | Effect |
|---|---|
| `@id` | primary key |
| `@default(autoincrement())` | serial key — `i64` / `BIGSERIAL`, database-filled |
| `@default(now())` | set on insert (`auto_now_add`) |
| `@updatedAt` | refreshed on every write (`auto_now`) |
| `@default(<literal>)` | a raw SQL default (`'active'`, `0`, `true`) |
| `@unique` | a unique column |
| `@index` | a secondary index |
| `@db.VarChar(n)` / `@db.Text` | a max length / an explicit text type |
| `@map("col")` | store under a different column name |
| `@relation(onDelete: Cascade\|Restrict\|SetNull\|NoAction)` | the foreign key's delete action |

## Block attributes

| Attribute | Effect |
|---|---|
| `@@map("table")` | rename the table |
| `@@id([a, b])` | a composite primary key |
| `@@unique([a, b])` | a composite unique constraint |
| `@@index([a, b])` | a composite index (built `CONCURRENTLY` by `migrate`/`push`) |
| `@@rag(…)` | opt the model into DORM's optional retrieval layer |

A column named by `@@id` / `@@unique` / `@@index` must be a real column of the
model, or `dorm validate` names the typo.

### A note on `@version`

Seiva's scaffold marks an optimistic-concurrency column with `@version`. DORM's
current schema parser does **not** accept that attribute: the data-layer sync
drops it and the integer `version` column is still created, but the concurrency
*enforcement* arrives when DORM recognizes the attribute. Treat `@version` today
as a convention plus a plain column, not an enforced constraint. See
[transactions](transactions.md).

## Enums

A top-level `enum` block declares a named set of string variants, compiled to a
**native database enum** — a `String` column backed by `CREATE TYPE … AS ENUM`:

```
enum OrderStatus {
  pending
  paid
  shipped
}

model Order {
  id     Int          @id @default(autoincrement())
  status OrderStatus
  note   OrderStatus?
}
```

The SQL type name is the snake_case of the enum name (`OrderStatus` →
`order_status`); the variants are emitted exactly as declared. The value is a
plain `String` in Rust, but the database rejects any value outside the set.
`dorm validate` rejects an enum with no variants, a duplicate variant, or a name
that collides with a model. Adding a variant to a live enum
(`ALTER TYPE … ADD VALUE`) is not yet generated as a migration step.

## The generated model

For a `model Author`, the derive macro emits:

- `impl Model for Author`, carrying its `&'static ModelMeta`;
- `Author::objects()` — the entry point for [queries](queries.md);
- `mod author` — one typed field handle, so `author::name.eq("ada")`;
- `struct NewAuthor` — the insert shape, carrying **only** caller-supplied
  columns. Columns the database fills in (a serial key, a `@default`, an
  `auto_now_add` stamp) are absent on purpose: there is no value for you to
  invent.

```rust
use dorm_core::prelude::*;
use dorm_derive::Model;

#[derive(Model, Debug, Clone)]
#[model(table = "authors", ordering = "name")]
struct Author {
    #[field(primary_key)]
    id: i64,
    #[field(unique, max_length = 120)]
    name: String,
}
```

A model cannot be generic and must have named fields — its metadata is one
`static`, and a tuple struct gives nothing to name a column after. The `#[model]`
attribute carries table-level choices (`table`, `ordering`, `unique_together`,
`check`, `tenant_scoped`); `#[field]` carries per-column choices mirroring the
schema attributes above (`primary_key`, `unique`, `index`, `max_length`,
`column`, `default`, `auto_now`, `foreign_key`, `on_delete`, `related_name`).

## What is not in the schema language yet

Table-level `@@check`, default-ordering, partial/expression indexes, and
multi-file schemas are roadmap. A composite `@@index` added to an *existing*
table is applied by `push`/`sync` but not yet emitted as a versioned incremental
migration step. `Cidr`/`MacAddr` work on a hand-written model but not yet from a
schema type. All of these are additive, and writing Rust models by hand remains
fully supported — the schema is simply the tooled path.

## Related

- [Making queries](queries.md) · [Relations](relations.md) ·
  [Migrations](migrations.md)
- The framework-level view: [Models and schema](../topics/data-layer/models.md)

---

*Part of the [Seiva documentation](../index.md).*
