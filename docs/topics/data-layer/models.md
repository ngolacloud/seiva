# Models and schema

A Seiva project's data shape lives in one file, `schema/<project>.schema`, in the
DORM schema DSL. Each in-process module owns a `namespace` block; the models
inside it are that module's tables.

## The schema file

`seiva project new` creates the schema with a datasource; `seiva module new <m>`
appends a `namespace <m>` block with a starter `Record` model:

```
datasource db {
  provider = "postgresql"
}

namespace notes {
  model Record {
    id      Uuid   @id
    title   String
    version Int    @version
  }
}
```

- **`namespace <module>`** — the module boundary. DORM supports namespaces
  natively and forbids relations across them, so a module's data cannot entangle
  with another's: only identifiers cross the boundary (the same rule the facade
  layer enforces on the API side).
- **`model`** — a table. Fields are `name Type [@attr …]`. Types include `Uuid`,
  `String`, `Int`, `BigInt`, `Boolean`, `DateTime`, `Decimal`, `Json`.
- **`@id`** — the primary key. **`@version`** marks the optimistic-concurrency
  column (the framework's convention; see the note below). Other attributes:
  `@unique`, `@index`, `@default(…)`, `@relation(…)`, `@db.VarChar(n)`.

## From schema to tables

`seiva db dev` rewrites `data/dorm.schema` from this file, generates the model
code, and authors a migration from it (see [migrations](migrations.md)). So the
schema is the single source of truth: edit it (or add a module), run
`seiva db dev`, and the migration follows.

Model names map to tables by DORM's convention (pluralised snake_case): the
`notes` module's `Record` becomes the `records` table, `id uuid`, `title text`,
`version` an integer column.

> **Note on `@version`.** The scaffold marks the concurrency column with
> `@version`. DORM's current schema parser does not accept that attribute, so the
> data-layer sync drops it (the `version` column is still created); the
> optimistic-concurrency *enforcement* arrives when DORM recognises the attribute.

## Brownfield

Already have a database? `seiva db pull` introspects it into model blocks you fold
into your modules' namespaces — see [introspection](../../reference/cli.md) and
the [data layer overview](index.md).

## Related

- [Migrations](migrations.md) · [Making queries](queries.md) · [The data layer](index.md)

---

*Part of the [Seiva documentation](../../index.md).*
