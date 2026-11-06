# Transactions

DORM gives you a **scope-based** unit of work: you run a closure inside a
transaction, and the result decides whether it commits. You never thread a
connection handle through your calls — DORM tracks the active transaction in a
task-local, so every query inside the scope runs on it automatically.

> In a Seiva application you reach this through the `UnitOfWork` *port*, not the
> `db::atomic` function directly. The last section maps the two; the framework
> view is in [Transactions and the unit of work](../topics/data-layer/transactions.md).

## `atomic`

`db::atomic` runs a closure inside a transaction, committing on `Ok` and rolling
back on `Err`:

```rust
use dorm_core::prelude::*;

db::atomic(|_tx| async move {
    let ada = Author::objects().create(NewAuthor { name: "Ada".into() }).await?;
    Post::objects()
        .create(NewPost { author: ada.as_fk(), slug: "on-engines".into() })
        .await?;
    Ok(())
})
.await?;
```

Everything inside runs on the transaction's connection — you do **not** pass a
handle to each call.

## Nesting is by savepoint

An `atomic` inside another becomes a `SAVEPOINT`: if the inner block fails it
undoes only its own work, and the outer transaction can still commit.

```rust
db::atomic(|_| async move {
    do_the_required_part().await?;

    // An optional part that may fail without losing the required part.
    let _ = db::atomic(|_| async move { do_the_optional_part().await }).await;

    Ok(())
})
.await?;
```

## What happens on a panic

An error return rolls back explicitly. A panic cannot `.await` a `ROLLBACK`, so
DORM instead takes the connection **out of the pool** on the way down — it is
closed rather than recycled, and PostgreSQL rolls back a transaction whose
connection drops. The same mechanism covers the process being killed: nothing a
scope began but did not commit is ever left written.

## Read-only mode

Mark the whole process read-only and every write refuses *before* it reaches SQL,
naming the operation it blocked:

```rust
dorm_core::db::set_read_only(true);

Post::objects().filter(post::id.eq(1)).delete().await?;
// Err: operation `delete` refused — the session is read-only
```

The session is *also* opened `READ ONLY` at the server (the real guard); the flag
exists so the refusal names the operation instead of surfacing a raw database
error. Both the [shell](cli-and-studio.md) and [Studio](cli-and-studio.md#dorm-studio)
share this guard, which is why they are read-only unless started with `--write`.

## Interactive transactions (the shell)

`atomic` brackets a single future. The shell (`dorm shell --write`) also exposes
transactions **interactively**, held open across commands:

```text
dorm*> :begin
dorm*> insert Author name=Ada
dorm*> insert Author name=Grace
dorm*> Author                 # the session sees its own uncommitted rows
dorm*> :commit                # …persisted atomically  (or :rollback to discard)
```

`:begin` opens a transaction on a dedicated connection; every following command
runs on it, so a read sees the uncommitted work and a failed statement aborts the
transaction. `:commit` persists everything at once; `:rollback` — or leaving the
shell with a transaction open — discards it.

## How Seiva's `UnitOfWork` maps onto DORM

Seiva's `UnitOfWork` port is scope-based for the same reason DORM's is: the
application expresses atomicity as a closure without naming the database, which
keeps use cases testable (a fake unit of work just runs the body).

```rust
unit_of_work.transaction(|| async {
    repository.save(&record).await?;                 // business change
    events.publish("notes.created", &data).await?;   // event, same tx
    Ok(())
}).await?;
```

The `seiva-data-dorm` gateway implements that port by mapping it straight onto
DORM's closure-scoped executor: a per-module session (`with_session` — strict
mode requires one) plus `db::atomic` (a real database transaction). The port's
boundary error is bridged to a DORM error so `atomic` rolls back on failure, and
back to the port's error type on the way out. A committing transaction persists;
a failing one leaves nothing behind.

### The transactional outbox

When DORM-backed, the `EventPublisher` port writes the event to a transactional
**outbox** *in the same transaction* as the business change — so the event and
the change commit together or not at all (no lost or phantom events). `publish`
therefore must be called inside `transaction`; the DORM outbox refuses a publish
outside one. A separate relay worker forwards outbox events to the broker
afterwards — see [Using DORM from Seiva](using-from-seiva.md#the-outbox-relay)
and the framework's [messaging guide](../topics/messaging-and-events.md).

## A note on optimistic concurrency

DORM has a design for an optimistic-concurrency column (bump a `version` on each
write, reject a stale write with a conflict), and Seiva's scaffold marks that
column with `@version`. The schema parser does not yet recognize the attribute,
so today the `version` column exists but the *enforcement* is not wired through
the schema path — treat it as pending. For an in-row atomic counter that is live
today, use an `F` expression in an `update` (see
[updating rows](queries.md#updating-rows)).

## Related

- [Making queries](queries.md) · [Raw SQL](raw-sql.md) ·
  [Backends](backends.md)
- [Using DORM from Seiva](using-from-seiva.md) ·
  [ADR 0004 — DORM gateway](../adr/0004-dorm-data-layer-gateway.md)

---

*Part of the [Seiva documentation](../index.md).*
