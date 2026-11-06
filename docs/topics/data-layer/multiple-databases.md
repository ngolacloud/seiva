# Multiple databases and backends

> This is Seiva's **multiple databases** topic — the story is shaped by DORM, the
> data layer.

## Seiva is database-agnostic at the seam

Nothing in Seiva hard-codes a database. The data commands pass a DSN straight
through to DORM, and the gateway's `connect` takes the DSN too:

- `seiva db dev` / `migrate` read `DELONIX_ORM_DSN`;
- `seiva db pull` / `studio` take `--dsn` (or `DELONIX_ORM_DSN`);
- the `seiva-data-dorm` gateway's `DormConfig.dsn` is the module's DSN.

So which database a Seiva service talks to is a deployment choice (the DSN), not a
code choice. The *reach* of that choice is set by DORM's runtime.

## Choosing a backend

`seiva db init --backend <postgres|mysql|sqlite>` (default `postgres`) sets the
DORM feature and the schema's `provider`; `seiva db dev`/`migrate` then target
that backend, reading the DSN from `DELONIX_ORM_DSN`.

## What works today

**PostgreSQL** is the primary, first-class backend.

**MySQL** works via DORM's `mysql` driver. Proven live end to end through Seiva:

```console
$ seiva db init --backend mysql
$ DELONIX_ORM_DSN='mysql://root:pw@127.0.0.1:3306/app' seiva db dev initial
```

created the module's `records` table (`id char(36)` primary key — `Uuid` maps to
`CHAR(36)` — `title text`, `version int`) and the migration ledger in MySQL 8,
from the module's model with no hand-written SQL.

**CockroachDB** works with **zero configuration beyond the DSN** — it is
wire-compatible with PostgreSQL, so the `postgres` backend serves it. Proven live:
`seiva db init` + `seiva db dev` against a CockroachDB node created the same
`records` table there. Any PostgreSQL-wire database (CockroachDB, and managed
Postgres such as Neon/Supabase) is reachable this way.

**SQLite** is available the same way (`--backend sqlite`), for local/embedded use.

## What is not live yet

**SQL Server, Oracle and MongoDB** are on DORM's roadmap (ADR 0011): the query
compiler carries dialect profiles, and the backend-trait (`Driver`) seam that made
MySQL/SQLite live extends to them as DORM adds each driver. When DORM ships a
driver, Seiva reaches it through the same `--backend`/DSN with no change here —
the Seiva side is already backend-agnostic.

## Related

- [The data layer (DORM)](index.md) · [Migrations](migrations.md)
- [Competitive landscape](../../competitive-landscape.md) (the multi-database row)

---

*Part of the [Seiva documentation](../../index.md).*
