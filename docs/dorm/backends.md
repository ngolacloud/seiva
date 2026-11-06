# Backends

DORM is backend-generic at the seam: the query compiler carries a `Dialect`, the
runtime seam is a `Driver` the `Database` holds, and each backend declares the
capabilities it supports so the core **degrades or refuses with a named error
rather than emitting silently-wrong SQL**. Which database a service talks to is a
deployment choice — the DSN — not a code change.

## The DSN-agnostic data path

Nothing in Seiva hard-codes a database. The data commands pass a DSN straight
through to DORM:

- `seiva db dev` / `seiva db migrate` read `DELONIX_ORM_DSN`;
- `seiva db pull` / `seiva db studio` take `--dsn` (or `DELONIX_ORM_DSN`);
- the `seiva-data-dorm` gateway's `DormConfig.dsn` is the module's DSN.

Within DORM itself, the DSN comes from the environment:

| Variable | Role |
|---|---|
| `DELONIX_ORM_DSN` | The DSN DORM uses. Checked first. |
| `PAAS_DATABASE_URL` | Accepted as a fallback when `DELONIX_ORM_DSN` is unset. |

Both the libpq keyword form and the URL form work; if neither variable is set the
first query returns a configuration error naming both, not a panic.

```sh
export DELONIX_ORM_DSN='host=/var/run/postgresql user=me dbname=blog'
export DELONIX_ORM_DSN='postgresql://me:secret@localhost:5432/blog'
```

> **In a Seiva application** the library reads **no** environment variables — the
> gateway hands DORM its config (DSN, replicas, pool, TLS, cursor key)
> programmatically per module. The environment DSN above is for the CLI and for a
> standalone DORM program. See
> [Using DORM from Seiva](using-from-seiva.md).

## Choosing a backend

`seiva db init --backend <postgres|mysql|sqlite>` (default `postgres`) sets the
DORM feature and the schema's `provider`; `seiva db dev`/`migrate` then target
that backend over the DSN.

## What works today

**PostgreSQL** is the primary, first-class backend — the target DORM is most
hardened against and the one every feature lands on first.

**MySQL** works through DORM's MySQL executor. The dialect differences are handled
for you: `?` placeholders, backtick quoting, no `RETURNING` (DORM reads back the
key via `last_insert_id`), `TINYINT(1)` booleans. Proven live end to end through
Seiva — `Uuid` maps to `CHAR(36)`, and the migration ledger is created in MySQL 8
from the module's model with no hand-written SQL.

**CockroachDB** works with **zero configuration beyond the DSN**: it is
PostgreSQL-wire-compatible, so the `postgres` backend serves it unchanged. Any
PostgreSQL-wire database reached this way — CockroachDB, and managed Postgres
offerings — needs no code change.

**SQLite** is available with `--backend sqlite`, for local and embedded use. Its
executor uses a reader pool with a single writer under WAL rather than one shared
connection.

## What is not live yet

**SQL Server, Oracle and MongoDB** are on DORM's multi-backend roadmap. The
groundwork is the dialect/driver/capability seam that made MySQL and SQLite live;
each new SQL backend is "implement the dialect arms + driver + type mapping +
capability declaration", localized rather than a scatter-edit. MongoDB is not SQL
and breaks the `Plan → SQL` model, so it is explicitly gated behind a spike and a
decision — it is *not* committed. When DORM ships a driver, Seiva reaches it
through the same `--backend`/DSN with no change on the Seiva side, which is
already backend-agnostic.

## TLS

TLS is chosen by the DSN's `sslmode`, or overridden by `DELONIX_ORM_SSLMODE` /
`DELONIX_ORM_SSLROOTCERT`. DORM never half-encrypts — the opportunistic libpq
modes (`prefer`, `allow`) and an absent `sslmode` mean plaintext; you ask for
`require` or `verify-full` to get TLS.

| `sslmode` | Meaning | Status |
|---|---|---|
| `disable` *(default)* | Plaintext. | supported |
| `require` | Encrypt; do **not** verify the certificate. | supported |
| `verify-full` | Encrypt **and** verify the chain and hostname. Roots from `sslrootcert`, else the bundled Mozilla roots. | supported |
| `verify-ca` | Verify the chain, not the hostname. | not yet — use `verify-full` or `require` |

```sh
export DELONIX_ORM_DSN='postgresql://me@db.internal:5432/blog?sslmode=verify-full&sslrootcert=/etc/ssl/ca.pem'
```

The TLS implementation is a Cargo feature of `dorm-core`: `tls-rustls` (default,
pure Rust, builds offline) or `tls-openssl` (system libssl). With neither
compiled in, a DSN that asks for TLS is a configuration error — never a silent
downgrade.

## Pooling and the one-runtime rule

DORM owns a connection pool, tuned by environment (for the CLI) or by
`ConnectOptions` (for a framework):

| Variable | Default | Meaning |
|---|---|---|
| `DELONIX_ORM_POOL_MAX` | 8 | Ceiling on pooled connections. |
| `DELONIX_ORM_POOL_ACQUIRE_TIMEOUT_MS` | 5000 | How long a caller waits for a connection before giving up. |
| `DELONIX_ORM_STATEMENT_TIMEOUT_MS` | 0 (off) | Server-side `statement_timeout`; a positive value cancels an over-running statement and returns `Error::Timeout`. It bounds DDL too, so leave it off while migrating unless you mean it. |

Because each connection is driven by a background task, a pooled connection
belongs to the tokio runtime that created it:

> **The process must keep one tokio runtime alive for as long as it uses DORM.**

In an async Seiva service this is automatic. A synchronous program driving DORM
through `block_on` must build its runtime **once** and reuse it — a runtime per
statement works for exactly one statement.

## Related

- [Transactions](transactions.md) · [Raw SQL](raw-sql.md) ·
  [Using DORM from Seiva](using-from-seiva.md)
- Framework view: [Multiple databases and backends](../topics/data-layer/multiple-databases.md)

---

*Part of the [Seiva documentation](../index.md).*
