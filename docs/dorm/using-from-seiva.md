# Using DORM from Seiva

DORM and Seiva are two products. Seiva *consumes* DORM, through one small gateway
crate and behind one feature flag, and the dependency is strictly one-way: Seiva
depends on DORM, never the reverse. This page shows the whole seam — how a Seiva
build turns DORM on, which crate holds the coupling, and how the `seiva db *`
commands relate to the `dorm` CLI.

## The public workspace is DORM-free by default

Seiva's core never depends on DORM. The framework entrypoint reports it plainly:
a build that does not enable the `dorm` feature carries no ORM, and
`seiva doctor` surfaces a `DORM_ENABLED` flag so an operator can see at a glance
which data layer a binary links.

This is deliberate. A module whose every repository talks to an external data
source should not drag in an ORM, and the default in-memory adapters let you
build and run a complete service **today with no database** — which is also what
keeps use cases unit-testable. Turning DORM off is simply *not compiling the
gateway crate*.

## The `dorm` feature and the gateway

All coupling to DORM lives in one crate — the `seiva-data-dorm` gateway —
compiled only when you enable the `dorm` feature. The rest of Seiva depends only
on the trait-only ports in `seiva-ports`. A module's `infrastructure` layer uses
the gateway to build its DORM handle and to satisfy the ports its `application`
declared.

```
application  ──(ports: UnitOfWork, EventPublisher, Clock, IdGenerator, RecordRepository)──▶
                                                                                      infrastructure
                                                                                        └─ seiva-data-dorm (feature = "dorm")
```

The gateway depends on DORM by published crate version and implements the four
core ports:

- **`Clock`** — the real system clock.
- **`IdGenerator`** — a UUIDv4-backed id generator.
- **`UnitOfWork`** — via `DormUnitOfWork`, mapping the scope-based port onto
  DORM's `with_session` + `db::atomic` (see
  [transactions](transactions.md#how-seivas-unitofwork-maps-onto-dorm)).
- **`EventPublisher`** — via `DormOutboxPublisher`, writing each event to the
  transactional outbox *in the current transaction*.

## `connect` — and the two invariants it enforces

A framework hands the gateway a `DormConfig` (per module: its DSN, read replicas,
PostgreSQL schema, credentials), mapped onto DORM's own `ConnectOptions`.
`connect(&DormConfig)` builds the module's `Database` and, crucially, enforces the
two boundary invariants a framework must not forget:

- **Strict sessions on** — no silent process-global `DEFAULT_DB`. A query with no
  session is a typed error, so every request runs on an explicit per-module
  session (`with_session`), not an ambient handle.
- **The keyset cursor key configured** — no forgeable default. The signing key
  for [keyset cursors](queries.md#keyset-pagination-for-lists) is set
  programmatically, so pagination cursors cannot be forged.

```rust
use seiva_data_dorm::{connect, DormConfig};

let db = connect(&DormConfig {
    dsn: module_dsn,
    replicas: vec![],
    // …per-module schema, pool, TLS
})?;
```

The DORM library itself reads **no** environment variables in this path — the
gateway supplies everything by API. (The `DELONIX_ORM_DSN` environment variable is
for the CLI lane below and for a standalone DORM program.)

## The outbox relay

The DORM-backed `EventPublisher` only writes to the outbox; something has to
forward those rows to the broker. That is the `seiva-outbox-relay` worker. One
pass drains a batch from the in-database outbox and publishes each event through a
messaging `Publisher`, **at-least-once**: the cursor advances only after a
successful publish, so a failed publish retries, and after a cap the event
dead-letters.

The division of labor matches DORM's boundary rule: *consuming* the in-database
outbox is DORM's (`dorm-outbox` offers a one-pass `relay`), while the **loop**
around it — interval, graceful shutdown, metrics, leader election — belongs to the
framework's worker runtime. This is why `publish` must be called inside a
`transaction`: the event and the business change commit together, and the relay
delivers afterward. See the framework's
[messaging and events](../topics/messaging-and-events.md) guide.

## The `seiva db *` commands

Day-to-day you do not call `dorm` directly; the `seiva` CLI wraps it. Each
command delegates to the `dorm` CLI (found on `PATH`, or via `SEIVA_DORM_BIN`):

| Command | Delegates to | Does |
|---|---|---|
| `seiva db init [--backend postgres\|mysql\|sqlite]` | `dorm init` | Scaffold the project's DORM data layer under `data/` (schema, migrations, the migrate binaries). Run once. Sets the DORM feature and the schema's `provider`. |
| `seiva db dev [name]` | `dorm migrate dev` | The dev loop: sync `data/dorm.schema` from your project schema, regenerate models, author + apply the next migration. |
| `seiva db migrate` | `dorm migrate` | The deploy lane: apply the recorded chain in order. Idempotent; authors nothing. |
| `seiva db pull [--out] [--dsn]` | `dorm db pull` | Brownfield: introspect a live database into model blocks you fold into your modules' `namespace`s. |
| `seiva db studio [--write] [--port] [--dsn]` | `dorm studio` | The browser data admin, read-only unless `--write`, loopback only. |

The DSN flows straight through — `seiva db dev`/`migrate` read `DELONIX_ORM_DSN`;
`pull`/`studio` take `--dsn` or fall back to it. See
[Migrations](migrations.md#the-seiva-workflow) for the authoring workflow and
[The CLI and Studio](cli-and-studio.md) for the `dorm` surface these wrap.

## Why the boundary is where it is

DORM is the data-source layer: one database, one transaction, one process. Things
whose guarantee comes from communication between services, from time, from caller
identity, or from policy are **not** the ORM's — they are Seiva's. That is why a
generated REST API, user accounts, broker clients, and long-running worker loops
that once lived in DORM are now the framework's responsibility (DORM ADR 0054),
and why the gateway, the relay loop, and the request-scoped session all live on
the Seiva side of the line. The result: a plain CLI user of DORM loses nothing,
and a Seiva service gets the ORM as a replaceable, well-fenced dependency.

## Related

- [Transactions](transactions.md) · [Migrations](migrations.md) ·
  [Backends](backends.md) · [The CLI and Studio](cli-and-studio.md)
- The data-layer topic guides:
  [overview](../topics/data-layer/index.md),
  [transactions](../topics/data-layer/transactions.md)
- [ADR 0004 — the DORM data-layer gateway](../adr/0004-dorm-data-layer-gateway.md)

---

*Part of the [Seiva documentation](../index.md).*
