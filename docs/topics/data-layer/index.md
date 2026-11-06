# The data layer (DORM)

> This is Seiva's **data layer** — where models, queries and persistence live.

Seiva's default data layer is **DORM** (the Delonix ORM), an **independent
product** wired into Seiva only through the optional `seiva-data-dorm` gateway,
behind the `dorm` feature. This separation is deliberate (ADR 0001, ADR 0004):

- The framework **core never depends on DORM**. A module whose every repository
  uses an external data source pulls in no ORM at all.
- The application layer codes against **trait-only ports** (`seiva-ports`:
  `UnitOfWork`, `EventPublisher`, `Clock`, `IdGenerator` and a module's own
  repository trait), never against a concrete database. That is what keeps a use
  case unit-testable with no database (see [Testing](../testing.md)).

```
application ──(RecordRepository port)──▶ infrastructure
                                            ├─ InMemoryRecordRepository   (default, DORM-free)
                                            └─ DormRecordRepository        (feature = "dorm")
```

## Why DORM (and why Rust)

Prisma — the reference TypeScript ORM — spent years shipping a native **Rust**
query engine, then in Prisma 7 (Nov 2025) replaced it with TypeScript/WASM,
because the **Rust↔JavaScript FFI boundary** was the bottleneck. DORM has no such
boundary: it is **native Rust end to end** — no GC, no FFI, compile-time type
safety through to the binary, and a single-binary deploy. The lesson Prisma
learned the hard way is DORM's starting point.

## On this page / in this section

- [Models and schema](models.md) — the schema DSL, `namespace` blocks, `@version`
- [Making queries](queries.md) — typed queries, keyset pagination, joins (no N+1)
- [Migrations](migrations.md) — evolving the schema safely
- [Transactions and the unit of work](transactions.md) — atomic writes, the outbox
- [Multiple databases and backends](multiple-databases.md) — routing, backend choice
- [Raw SQL escape hatch](raw-sql.md) — typed, parameterised raw queries

## Status

DORM is a working, independent product (its own repository, 130+ tests). The
Seiva ↔ DORM cutover and the feature-gated gateway are in progress; the default
in-memory adapters let you build and run a complete service today with no
database. Track it in the [engineering log](../../progress.md) and
[ADR 0004](../../adr/0004-dorm-data-layer-gateway.md).

---

*Part of the [Seiva documentation](../../index.md).*
