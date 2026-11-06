# Migrations

Seiva's migrations are DORM's, driven from your project's schema. Because a Seiva
project is DORM-compatible (`seiva db init` sets up the data layer under `data/`),
the workflow matches Prisma's `migrate dev`/`deploy`: an author step that
generates the next migration and a deploy step that applies the recorded chain.

## Set up the data layer once

```console
$ seiva db init          # scaffolds data/ (dorm.schema, migrations/, the lanes)
```

`db init` writes `data/dorm.schema` from your project's `schema/<project>.schema`
— the module `namespace`s and their models — and generates the model code. Your
project schema is the source of truth; `data/` is the migration engine.

## The dev loop

```console
$ seiva module new notes       # adds a `namespace notes { model Record … }`
$ seiva db dev add_notes       # sync schema → generate → author + apply migration
```

`seiva db dev <name>` does three things in order:

1. **sync** — rewrites `data/dorm.schema` from `schema/<project>.schema`, so every
   module's models are current;
2. **generate** — regenerates the model code from that schema;
3. **migrate dev** — diffs the models against the recorded history in
   `data/migrations/`, writes the new migration, and applies it.

It needs a database: set `DELONIX_ORM_DSN` (PostgreSQL, or any PG-wire engine such
as CockroachDB — see [multiple databases](multiple-databases.md)).

## Deploy

```console
$ seiva db migrate       # apply the recorded chain (prisma migrate deploy)
```

`seiva db migrate` applies the migrations already authored in `data/migrations/`,
in order, recording each in the `orm_schema_migrations` ledger. It authors
nothing and needs no schema sync — it is the production lane, safe to run on every
deploy (it is idempotent: already-applied migrations are skipped).

## What is recorded

- `data/migrations/NNNN_<name>.json` — the ordered migration files (the SQL, a
  reversible `down`, and a schema snapshot). Commit these.
- `orm_schema_migrations` (in the database) — the ledger of applied versions.

## Status and rollback

The underlying DORM lanes also offer `migrate status` (what the DB has applied),
`migrate down <version>` (reverse, when the migration is reversible) and a
rolling-upgrade safety lint; these are available through the `dorm` CLI in
`data/` and will surface as `seiva db` subcommands as the workflow matures.

## Related

- [Models and schema](models.md) · [The data layer](index.md)
- [Multiple databases](multiple-databases.md) · [The `seiva` CLI](../../reference/cli.md)

---

*Part of the [Seiva documentation](../../index.md).*
