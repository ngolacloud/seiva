//! The DORM data-layer gateway (Parte C; Seiva ADR 0004).
//!
//! All coupling to DORM lives here. The rest of Seiva depends only on
//! `seiva-ports`; a module's `infrastructure` crate uses this gateway to build
//! its DORM handle and to satisfy the ports its `application` declared. Turning
//! the DORM data layer off is simply not compiling this crate (the brief's Parte
//! C), which is why the coupling is confined to one place.
//!
//! This first cut crosses the boundary and proves it: it compiles against DORM
//! pinned by git revision, implements the ports that map cleanly ([`Clock`],
//! [`IdGenerator`]), and offers [`connect`], which builds a module's `Database`
//! while enforcing the two boundary invariants a framework must not forget —
//! strict sessions on (no silent global `DEFAULT_DB`) and the keyset cursor key
//! configured (no forgeable default). `UnitOfWork` and `EventPublisher` are
//! deferred: DORM's transaction API is closure-scoped (`db::atomic`) while the
//! `seiva-ports` `UnitOfWork` is handle-based (begin/commit), a mismatch Seiva
//! ADR 0004 records and resolves next (a scope-based port, or DORM's `on_commit`
//! primitive, DORM-B3).

#![forbid(unsafe_code)]

use seiva_ports::{Clock, Id, IdGenerator};
use std::time::SystemTime;

/// The real system clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// A UUIDv4-backed id generator. The 16 random bytes fill [`Id`] directly, so
/// the framework and adapters render it as a UUID.
pub struct UuidIdGenerator;

impl IdGenerator for UuidIdGenerator {
    fn new_id(&self) -> Id {
        Id(*uuid::Uuid::new_v4().as_bytes())
    }
}

/// The data-layer configuration a framework hands the gateway, mapped onto
/// DORM's own [`dorm_core::db::ConnectOptions`]. Per module: its own DSN,
/// replicas, PostgreSQL schema and credentials.
#[derive(Debug, Clone)]
pub struct DormConfig {
    /// The primary DSN.
    pub dsn: String,
    /// Read-replica DSNs, if any.
    pub replicas: Vec<String>,
    /// The default read-only flag for sessions on this handle.
    pub read_only: bool,
    /// The PostgreSQL schema (namespace) this module's handle is bound to.
    pub schema: Option<String>,
    /// Maximum pooled connections.
    pub pool_max: usize,
    /// The HMAC key for opaque keyset cursors (from the framework's secrets).
    pub cursor_key: Vec<u8>,
}

/// Builds a module's DORM [`Database`](dorm_core::db::Database) from `config`,
/// and enforces the boundary invariants DORM-B1 established: **strict sessions**
/// (a statement with no session in scope is an error, never the process-global
/// default) and the **cursor signing key** (no forgeable default). The returned
/// handle is driven per request through `dorm_core::db::with_session`.
pub fn connect(config: &DormConfig) -> Result<dorm_core::db::Database, dorm_core::Error> {
    // A framework always runs through explicit per-module, per-request sessions.
    dorm_core::db::set_strict_sessions(true);
    dorm_core::cursor::set_cursor_key(config.cursor_key.clone());

    let options = dorm_core::db::ConnectOptions {
        pool_max: config.pool_max,
        ..dorm_core::db::ConnectOptions::default()
    };
    let replicas: Vec<&str> = config.replicas.iter().map(String::as_str).collect();
    dorm_core::db::Database::connect_with(
        &config.dsn,
        &replicas,
        config.read_only,
        config.schema.as_deref(),
        &options,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_ids_are_distinct() {
        let g = UuidIdGenerator;
        assert_ne!(g.new_id(), g.new_id());
    }

    #[test]
    fn system_clock_reads_a_time() {
        assert!(SystemClock.now() >= std::time::UNIX_EPOCH);
    }

    #[test]
    fn connect_builds_a_handle_and_enforces_the_invariants() {
        // deadpool is lazy, so a parseable DSN builds the handle without
        // connecting — enough to prove the gateway links against real DORM and
        // calls connect_with + strict sessions + the cursor key.
        let cfg = DormConfig {
            dsn: "host=localhost user=u dbname=x sslmode=disable".into(),
            replicas: vec![],
            read_only: false,
            schema: Some("notes".into()),
            pool_max: 4,
            cursor_key: b"test-secret".to_vec(),
        };
        let db = connect(&cfg).expect("the handle builds without connecting");
        assert_eq!(db.replica_count(), 0);
        assert!(dorm_core::db::strict_sessions(), "strict sessions must be on");
    }
}
