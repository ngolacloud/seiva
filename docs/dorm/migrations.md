# Migrations

DORM generates schema from your models' metadata — the same `ModelMeta` the
queries read — and applies it through a small, versioned migrator. You always get
to **see the SQL before it runs**.

This page covers both views: the `seiva db *` workflow you use day to day, and
the DORM migrator underneath it.

## The Seiva workflow

A Seiva project is DORM-compatible: `seiva db init` sets up the data layer under
`data/`, and the `seiva db dev` / `seiva db migrate` pair drives DORM's authoring
and deploy lanes.

```console
$ seiva db init                 # scaffold data/ (dorm.schema, migrations/, the lanes)
$ seiva module new notes        # adds a `namespace notes { model Record … }`
$ seiva db dev add_notes        # sync schema → generate → author + apply a migration
$ seiva db migrate              # deploy: apply the recorded chain (idempotent)
```

`seiva db dev <name>` does three things in order:

1. **sync** — rewrites `data/dorm.schema` from your project's
   `schema/<project>.schema`, so every module's models are current;
2. **generate** — regenerates the model code from that schema;
3. **migrate dev** — diffs the models against the recorded history in
   `data/migrations/`, writes the new migration, and applies it.

`seiva db migrate` is the production lane: it applies the migrations already
authored in `data/migrations/`, in order, recording each in the ledger. It
authors nothing and needs no schema sync, so it is safe to run on every deploy.
Both commands read the DSN from `DELONIX_ORM_DSN` (PostgreSQL, or any PG-wire
engine; see [backends](backends.md)). These commands delegate to the `dorm` CLI —
see [Using DORM from Seiva](using-from-seiva.md).

> Seiva's framework-level migrations guide is
> [here](../topics/data-layer/migrations.md); this page is the DORM detail under
> it.

## Generating DDL

Underneath, `dorm-migrations` turns models into `CREATE TABLE` / `CREATE INDEX`
text. Every statement is emitted with `IF NOT EXISTS`, which is what lets the
migrator *adopt* a database that already carries the schema:

```rust
use dorm_core::model::Model;
use dorm_migrations::ddl;

let models = [<Author as Model>::meta(), <Post as Model>::meta()];
println!("{}", ddl::schema_for(&models)?);          // every table and index
println!("{}", ddl::create_table(<Post as Model>::meta())?);  // one table alone
```

## The initial migration and the `Migration` value

`initial` packages the whole generated schema as **version 1**. A `Migration` is
a plain, inspectable value:

```rust
use dorm_migrations as mig;

let m = mig::initial(&models)?;
assert_eq!(m.version, 1);

// pub struct Migration { pub version: i64, pub name: String, pub statements: Vec<String> }
```

> **One ledger, one set of models.** `initial` always mints version 1. Do not
> compose two independent model sets into the same migration history — give each
> its own database/ledger, or fold them into one model set.

## Applying migrations

`migrate` applies every version not yet recorded, in order, under a PostgreSQL
advisory lock, each inside the transaction that records it:

```rust
use dorm_migrations as mig;

// Returns the versions applied this call — empty when everything was current.
let applied = mig::migrate(&[mig::initial(&models)?]).await?;
```

It is idempotent in two ways: a version already in the ledger is skipped, and
because the statements are `IF NOT EXISTS`, a first run against a database that
already has the schema records the version **without altering anything**. Calling
`migrate` on every process start is therefore safe.

## The ledger

Applied versions are recorded in a table named **`orm_schema_migrations`**, which
the migrator creates itself on first run:

| column | meaning |
|---|---|
| `version` | the migration's version (primary key) |
| `name` | its name (`"initial"`, …) |
| `applied_unix` | wall-clock seconds when it was applied |

In a Seiva project the authored migrations are JSON files under
`data/migrations/NNNN_<name>.json` — the SQL, a reversible `down`, and a schema
snapshot. Commit those; the ledger lives in the database.

## Evolving the schema over time

The migrator applies a **list** of migrations by version. To change the schema
later, append a new `Migration` with the next version — **never edit an
already-applied one**:

```rust
let v2 = mig::Migration {
    version: 2,
    name: "add-posts-views".into(),
    statements: vec![
        "ALTER TABLE posts ADD COLUMN IF NOT EXISTS views BIGINT NOT NULL DEFAULT 0".into(),
    ],
};
mig::migrate(&[mig::initial(&models)?, v2]).await?;
```

Editing the SQL of a step a database already recorded does nothing to that
database — you would get two databases at the same version with different schemas,
the exact failure a version table exists to prevent. A wrong step is fixed by a
**new** step.

## `push` vs the migration chain

Two authoring paths exist, for two situations:

- **`dorm db push`** (`seiva`'s sync lane) syncs the database to the models
  *additively* — it never drops or retypes a column, which **is** its data-loss
  protection. It is for development iteration, not production history.
- **The migration chain** (`migrate dev` to author, `migrate deploy` to apply) is
  the reviewed, versioned history you ship.

The autodetector that authors a migration **refuses an ambiguous rename** — a
`DROP` of one column plus an `ADD` of another of the same type, which would
silently lose data — until you declare the rename (`@map`) or split it into two
migrations. Different types are still allowed.

## Status, rollback, and recovery

- `dorm migrate status` (`--json`) lists the applied-versions ledger with
  readable UTC timestamps.
- `dorm migrate diff` shows the additive changes `push` would apply, without
  touching the database.
- `dorm migrate down <version>` reverses a migration when it is reversible.
- `dorm migrate resolve <version>` marks a version applied in the ledger
  **without running its SQL** — recovery after a manual fix. It refuses unless you
  name the exact version and refuses a version already recorded.

These surface through the `dorm` CLI in `data/`, and are being lifted into
`seiva db` subcommands as the workflow matures.

## Related

- [Schema and models](schema-and-models.md) · [Transactions](transactions.md)
- [Backends](backends.md) · [The CLI and Studio](cli-and-studio.md)

---

*Part of the [Seiva documentation](../index.md).*
