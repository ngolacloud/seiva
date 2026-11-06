# Raw SQL

When the typed [query builder](queries.md) does not express what you need, DORM
provides a **parameterized** raw-SQL valve. It is a deliberate, auditable escape
hatch — not a back door for string-built SQL.

## When to reach for it

- A query the typed `QuerySet` cannot represent (a complex CTE, a
  database-specific function, a hand-tuned plan).
- A one-off maintenance or reporting statement.

Prefer the typed layer for everything else: it is compile-time checked and
portable across the [supported backends](backends.md). Raw SQL is the exception,
and it reads as one in a review.

## Always parameterized

Values are bound as parameters — `$1`, `$2`, … on PostgreSQL; `?` on MySQL (the
dialect decides) — and **never** spliced into the SQL string. That is the rule
that keeps the valve injection-safe:

```rust
use dorm_core::prelude::*;

// read: values travel as bound parameters, not string interpolation
let rows = db::sql(
    "SELECT id, title FROM records WHERE title = $1",
    &[Value::from(title)],
).await?;
```

Reads go through `db::sql` (returning rows of `Value`); writes through
`db::sql_execute` (returning the affected count). The CLI exposes the same valve
as `dorm db execute "<SQL>" -p <value>… [--write]`, where every `-p` value is
bound as `$n`, never spliced, and the result is rendered as the closed set of
types the ORM knows (project anything else as `::text`).

## Read-only by default

The raw valve is read-only unless you opt in: a write needs the explicit write
path (`db::sql_execute`, or `dorm db execute --write`) and still passes the
[read-only guard](transactions.md#read-only-mode). A forgotten flag cannot mutate.
On the CLI, a `--write` statement that is a bare `DROP`/`TRUNCATE` or a `DELETE`
without a `WHERE` confirms first (or needs `--yes`); targeted writes do not
prompt.

## It still respects the boundary

Raw SQL runs through the **same session and transaction scope** as the typed
queries, so it participates in the active [transaction](transactions.md) (and
therefore the `UnitOfWork` when called from a Seiva use case) and in any
primary/replica routing. It is an escape hatch from the *query builder*, not from
the framework's data discipline — the transaction, the read-only guard, and the
session all still apply.

> DORM deliberately scopes this to **local** raw SQL. A remote "named SQL"
> surface that other services could call was considered and rejected
> (DORM ADR 0058): cross-service access is contracts and events, not a shared SQL
> endpoint.

## Related

- [Making queries](queries.md) · [Transactions](transactions.md) ·
  [Backends](backends.md)
- Framework view: [Raw SQL escape hatch](../topics/data-layer/raw-sql.md)

---

*Part of the [Seiva documentation](../index.md).*
