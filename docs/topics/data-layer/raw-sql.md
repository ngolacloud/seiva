# Raw SQL escape hatch

When the typed query layer does not express what you need, DORM provides a
**parameterised** raw-SQL valve. It is deliberate and auditable (ADR 0005), never
a back door for string-built SQL.

## When to reach for it

- A query the typed `QuerySet` cannot represent (a complex CTE, a
  database-specific function, a hand-tuned plan).
- A one-off maintenance or reporting statement.

Prefer the typed layer for everything else: it is compile-time checked and
dialect-portable across the [supported backends](multiple-databases.md).

## Always parameterised

Values are bound as parameters (`$1`, `$2`, … on PostgreSQL; `?` on MySQL — the
dialect decides), **never** spliced into the SQL string. This is the rule that
keeps the valve injection-safe:

```rust
// values travel as bound parameters, not string interpolation
db::fetch("SELECT id, title FROM records WHERE title = $1", &[Value::from(title)]).await?;
```

A read-only posture is the default; a write requires an explicit opt-in, so a
forgotten flag cannot mutate.

## It still respects the boundary

Raw SQL runs through the same session/transaction scope as the typed queries, so
it participates in the active `UnitOfWork` ([transactions](transactions.md)) and
the primary/replica routing — it is an escape hatch from the *query builder*, not
from the framework's data discipline.

## Related

- [Making queries](queries.md) · [Transactions](transactions.md) · [Multiple databases](multiple-databases.md)

---

*Part of the [Seiva documentation](../../index.md).*
