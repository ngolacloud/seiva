# Making queries

A module's `application` layer queries through its **repository port**, not a
global database handle — so the use case stays testable and the SQL stays in the
adapter (DORM).

## Queries live behind a port

The application declares what it needs as a trait; the `infrastructure` layer
implements it (in-memory by default, DORM-backed with the `dorm` feature):

```rust
pub trait RecordRepository {
    fn save(&self, record: &Record) -> impl Future<Output = PortResult<()>> + Send;
    // add the reads the use case needs: find_by_id, list, …
}
```

The use case calls the port; it never names DORM. That is what lets Part 5's unit
tests run with a fake repository and no database.

## Typed queries through DORM

The DORM-backed adapter builds its reads with DORM's typed `QuerySet` — a
compile-time-checked query the pure compiler lowers to SQL for the active backend
([multiple databases](multiple-databases.md)). You express the query in Rust;
DORM produces the dialect.

## Pagination

Prefer **keyset** pagination (a cursor over a stable ordering) to offset
pagination for lists — it is stable under writes and does not degrade on deep
pages. The `list` route convention (see
[controllers and routing](../transport/controllers-and-routing.md)) carries the
query parameters; the repository turns them into a bounded, ordered read.

## No N+1

DORM resolves related data with SQL joins, not a query per row — so loading a
collection with its relations is one round-trip, not N+1. (This is a point where
DORM differs from ORMs that historically fetched relations separately.)

## Raw SQL, when you need it

For a query the typed layer does not express, drop to the parameterised raw-SQL
valve — never string-concatenate values. See [raw SQL](raw-sql.md).

## Related

- [Models and schema](models.md) · [Transactions](transactions.md) · [Raw SQL](raw-sql.md)
- [The data layer](index.md)

---

*Part of the [Seiva documentation](../../index.md).*
