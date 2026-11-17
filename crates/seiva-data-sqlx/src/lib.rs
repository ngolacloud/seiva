//! The sqlx reference data-layer adapter (ADR 0030, Fatia 2).
//!
//! This crate implements the `seiva-ports` service ports over a **bare query
//! layer** — `sqlx::Transaction<'_, Postgres>` — with no ORM in between. It is the
//! living proof that the Seiva data-layer boundary is agnostic even of an ORM:
//! everything the framework needs (a transaction, an outbox, idempotency, audit)
//! is expressible in SQL the user already runs. The user brings their own schema
//! and migrates with `sqlx-cli`; the DDL helpers here (`*_migrations`) cover only
//! the framework's own bookkeeping tables.
//!
//! ## The ambient transaction (the non-trivial part)
//!
//! [`UnitOfWork::transaction`](seiva_ports::UnitOfWork::transaction) is
//! **scope-based, not handle-based**: the body runs
//! inside the transaction and receives **no** handle (ADR 0030). So the module's
//! repositories cannot be passed a `&mut Transaction`; they find the current one
//! through a task-local this crate owns. [`SqlxUnitOfWork`]'s `transaction` opens a
//! `pool.begin()` transaction, stashes it in the `CURRENT` task-local for the
//! duration of the body, commits on `Ok` and rolls back on `Err`. A repository (or
//! the outbox / idempotency port) reaches the live transaction with
//! [`with_current_tx`]; [`current_scope`] exposes the tenant
//! [`Scope`](seiva_ports::Scope) the unit of
//! work was opened for.
//!
//! Unlike the DORM gateway, this adapter does **not** auto-enforce tenant scoping
//! (a bare query layer has no model metadata to filter on): it *surfaces* the
//! [`Scope`](seiva_ports::Scope) via [`current_scope`] so a tenant-aware repository can add its own
//! `WHERE tenant = $n` predicate, but the SQL is the user's. That is the honest
//! contract of "bring your own query layer".

#![forbid(unsafe_code)]

#[cfg(feature = "sqlx")]
mod adapter;

#[cfg(feature = "sqlx")]
pub use adapter::{
    audit_migrations, connect, current_scope, idempotency_migrations, outbox_migrations,
    with_current_tx, SqlxAuditSink, SqlxConfig, SqlxIdempotencyStore, SqlxOutboxPublisher,
    SqlxUnitOfWork,
};
