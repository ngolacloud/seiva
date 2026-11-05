# Part 3: Persisting with the data layer

So far `polls` keeps its questions and choices in memory. Django's
[Part 2](https://docs.djangoproject.com/en/6.1/intro/tutorial02/) sets up the
database, `migrate`s the models and uses the ORM. In this part we do the same with
the DORM data layer — against PostgreSQL, MySQL or any PG-wire database.

## Set up the data layer

```console
$ seiva db init                 # PostgreSQL (default); or --backend mysql | sqlite
```

`db init` scaffolds `data/` (the DORM project: schema, migrations, the migrate
binaries), writes `data/dorm.schema` from your project's
`schema/polls-site.schema` — so the module `namespace`s and their models drive it —
and sets the datasource provider for the chosen backend.

## Author and apply a migration

Point `DELONIX_ORM_DSN` at your database and run the dev loop (the Seiva
equivalent of `makemigrations` + `migrate`):

```console
$ export DELONIX_ORM_DSN="host=localhost user=me dbname=polls sslmode=disable"
$ seiva db dev initial
generated src/models.rs from dorm.schema (2 model(s))
created migration 1 (initial) — 2 statement(s) in migrations/
applied version(s): [1]
```

That created the `polls` module's `questions` and `choices` tables (each with
`id`, its text, and the choice's `votes` tally and `question_id`) and the
migration ledger. `seiva db dev <name>` always syncs the schema, regenerates the
models, and authors+applies the next migration; `seiva db migrate` deploys the
recorded chain (production). See [Migrations](../../topics/data-layer/migrations.md).

## A different database — same commands

The data path is DSN-agnostic. For MySQL:

```console
$ seiva db init --backend mysql
$ DELONIX_ORM_DSN='mysql://root:pw@127.0.0.1:3306/polls' seiva db dev initial
```

PostgreSQL, CockroachDB (PG-wire) and MySQL are all live backends — see
[multiple databases](../../topics/data-layer/multiple-databases.md).

## Wire the DORM adapter (next step)

The module still uses the in-memory `PollRepository` in the composition root;
swapping it for the DORM-backed adapter (behind the `dorm` feature) is the
production wiring — covered in
[the data layer guide](../../topics/data-layer/index.md). The use cases do not
change: they already code against the `PollRepository` port, and one transaction
per use case (ADR 0004) means a vote and its `polls.voted` event commit together
or not at all.

## Brownfield

Already have a database? `seiva db pull` introspects it into model blocks to fold
into your modules — see [the CLI reference](../../reference/cli.md).

## Next

[Part 4: Authentication and permissions](part-4-authentication.md).

---

*Part of the [Seiva documentation](../../index.md).*
