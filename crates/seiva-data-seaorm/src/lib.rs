//! The SeaORM reference data-layer adapter (ADR 0030, Fatia 2).
//!
//! This crate implements the `seiva-ports` service ports over SeaORM's
//! `DatabaseTransaction`. It is the direct answer to Loco — the same ORM, under a
//! framework that does not weld you to it. The user brings their own schema and
//! migrates with `sea-orm-cli`; the DDL helpers here (`*_migrations`) cover only
//! the framework's own bookkeeping tables (outbox, idempotency, audit).
//!
//! ## The ambient transaction (the non-trivial part)
//!
//! [`UnitOfWork::transaction`](seiva_ports::UnitOfWork::transaction) is
//! **scope-based, not handle-based** (ADR 0030): the
//! body receives no transaction handle, so a module's repositories find the current
//! one through a task-local this crate owns. [`SeaOrmUnitOfWork`]'s `transaction` opens
//! `conn.begin()`, stashes the owned [`DatabaseTransaction`](sea_orm::DatabaseTransaction)
//! in the `CURRENT` task-local for the body's duration, commits on `Ok` and rolls
//! back on `Err`. A repository (or the outbox / idempotency port) reaches the live
//! transaction with [`with_current_tx`]; [`current_scope`] exposes the
//! [`Scope`](seiva_ports::Scope).
//!
//! Like the sqlx adapter and unlike the DORM gateway, this adapter does **not**
//! auto-enforce tenant scoping — it surfaces the [`Scope`](seiva_ports::Scope) so a tenant-aware
//! repository can constrain its own query; the SQL is the user's.

#![forbid(unsafe_code)]

#[cfg(feature = "seaorm")]
mod adapter;

#[cfg(feature = "seaorm")]
pub use adapter::{
    audit_migrations, connect, current_scope, idempotency_migrations, outbox_migrations,
    with_current_tx, SeaOrmAuditSink, SeaOrmConfig, SeaOrmIdempotencyStore, SeaOrmOutboxPublisher,
    SeaOrmUnitOfWork,
};
