# Part 3: Persisting with the data layer

So far `notes` persists in memory. In this part we set up the DORM data layer and
turn the module's schema into a real table — against PostgreSQL, MySQL or any
PG-wire database.

## Set up the data layer

```console
$ seiva db init                 # PostgreSQL (default); or --backend mysql | sqlite
```

`db init` scaffolds `data/` (the DORM project: schema, migrations, the migrate
binaries), writes `data/dorm.schema` from your project's
`schema/notes-app.schema` — so the module `namespace`s and their models drive it —
and sets the datasource provider for the chosen backend.

## Author and apply a migration

Point `DELONIX_ORM_DSN` at your database and run the dev loop:

```console
$ export DELONIX_ORM_DSN="host=localhost user=me dbname=notes sslmode=disable"
$ seiva db dev initial
generated src/models.rs from dorm.schema (1 model(s))
created migration 1 (initial) — 1 statement(s) in migrations/
applied version(s): [1]
```

That created the `notes` module's `records` table (`id`, `title`, `version`) and
the migration ledger. `seiva db dev <name>` always syncs the schema, regenerates
the models, and authors+applies the next migration; `seiva db migrate` deploys the
recorded chain (production). See [Migrations](../../topics/data-layer/migrations.md).

## A different database — same commands

The data path is DSN-agnostic. For MySQL:

```console
$ seiva db init --backend mysql
$ DELONIX_ORM_DSN='mysql://root:pw@127.0.0.1:3306/notes' seiva db dev initial
```

PostgreSQL, CockroachDB (PG-wire) and MySQL are all live backends — see
[multiple databases](../../topics/data-layer/multiple-databases.md).

## Wire the DORM adapter (next step)

The module still uses the in-memory repository in the composition root; swapping it
for the DORM-backed adapter (behind the `dorm` feature) is the production wiring —
covered in [the data layer guide](../../topics/data-layer/index.md).

## Brownfield

Already have a database? `seiva db pull` introspects it into model blocks to fold
into your modules — see [the CLI reference](../../reference/cli.md).

## Next

[Part 4: Authentication and permissions](part-4-authentication.md).

---

*Part of the [Seiva documentation](../../index.md).*
