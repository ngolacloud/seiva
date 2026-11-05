# Multiple databases and backends

> This is the Seiva equivalent of Django's **multiple databases** topic — but the
> story is shaped by DORM, the data layer.

## Seiva is database-agnostic at the seam

Nothing in Seiva hard-codes a database. The data commands pass a DSN straight
through to DORM, and the gateway's `connect` takes the DSN too:

- `seiva db dev` / `migrate` read `DELONIX_ORM_DSN`;
- `seiva db pull` / `studio` take `--dsn` (or `DELONIX_ORM_DSN`);
- the `seiva-data-dorm` gateway's `DormConfig.dsn` is the module's DSN.

So which database a Seiva service talks to is a deployment choice (the DSN), not a
code choice. The *reach* of that choice is set by DORM's runtime.

## What works today

**PostgreSQL** is the primary, first-class backend.

**CockroachDB** works too, with **zero code changes** — it is wire-compatible with
PostgreSQL, so the same `tokio-postgres` path serves it. This is proven live:
pointing `DELONIX_ORM_DSN` at a CockroachDB node, `seiva db init` then
`seiva db dev` authored and applied a migration and created the module's table
(`records` with `id uuid, title text, version bigint`) and the migration ledger —
exactly as against PostgreSQL. Any PostgreSQL-wire database (CockroachDB, and
managed Postgres such as Neon/Supabase) is reachable the same way.

```console
# a CockroachDB single-node, for example
$ DELONIX_ORM_DSN="host=localhost port=26257 user=root dbname=app sslmode=disable" \
    seiva db dev initial
```

## What does not work yet — and why

**MySQL, SQL Server, Oracle and MongoDB are not live backends yet.** DORM's query
*compiler* already carries their SQL **dialect** profiles (ADR 0011), but DORM's
async *runtime* (`dorm-core`'s `db`: the pool, the executor, transactions) is
wired directly to the PostgreSQL driver. The remaining step — "one `db` over a
backend trait", the driver seam DORM's `dorm-sqlite` crate calls out — is a
DORM-side piece of ADR 0011. Until it lands, non-PostgreSQL-wire engines compile
their SQL but cannot be executed through the async runtime, so Seiva cannot drive
them live. This is a DORM capability, not a Seiva limitation: the moment DORM
executes a dialect, Seiva reaches it through the same DSN with no change here.

## Related

- [The data layer (DORM)](index.md) · [Migrations](migrations.md)
- [Competitive landscape](../../competitive-landscape.md) (the multi-database row)

---

*Part of the [Seiva documentation](../../index.md).*
