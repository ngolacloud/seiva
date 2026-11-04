//! Trait-only ports the application layer codes against.
//!
//! A module's `application` crate depends on this crate — never on
//! `delonix-seiva` — to declare the outbound capabilities a use case needs.
//! The `infrastructure` crate wires concrete implementations (DORM-backed or
//! external); the framework resolves them at composition time. Keeping this
//! crate free of any runtime, macro or data layer is what lets the application
//! layer stay independent of both the framework and the ORM.
//!
//! This is the first cut of the boundary; its exact shape is fixed by the Seiva
//! core ADR. The four ports below are the ones every module relies on:
//! [`UnitOfWork`], [`EventPublisher`], [`Clock`] and [`IdGenerator`].

#![forbid(unsafe_code)]

use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::time::SystemTime;

/// The error any port may surface, kept opaque and dependency-free: a boxed
/// std error so this crate carries no error library. Adapters map their
/// concrete errors (a DB error, a broker error) into this at the boundary, so
/// the application layer sees one uniform failure type.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Result returned by every port.
pub type PortResult<T> = Result<T, BoxError>;

/// A 128-bit opaque identifier. Rendered as a UUID or ULID by the framework and
/// by adapters; this crate stays free of a UUID dependency so the boundary
/// imposes no identifier library on downstream code.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Id(pub [u8; 16]);

impl std::fmt::Debug for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Id({})", hex16(&self.0))
    }
}

fn hex16(bytes: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}

/// Reads the current time. Injected so use cases are deterministic under test:
/// a fake clock returns a fixed instant with no real time involved.
pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
}

/// Mints new identifiers. Injected so a test double can hand out a predictable
/// sequence instead of random values.
pub trait IdGenerator: Send + Sync {
    fn new_id(&self) -> Id;
}

/// Publishes a domain event. The payload is already serialized by the caller
/// (the interface layer owns the envelope format, e.g. CloudEvents); this port
/// only moves bytes to a topic, so no serialization library rides in here.
///
/// Whether the publish is transactional (outbox in the same transaction) or
/// best-effort is a property of the implementation and of `seiva.toml`, not of
/// this trait: the application layer asks to publish and the boundary decides
/// how the guarantee is kept.
#[async_trait]
pub trait EventPublisher: Send + Sync {
    async fn publish(&self, topic: &str, payload: &[u8]) -> PortResult<()>;
}

/// The transaction boundary — **scope-based** (ADR 0002, amended by ADR 0004 of
/// the data-layer gateway).
///
/// The application decides where a transaction begins and ends, one per use
/// case, by running the use-case body inside [`transaction`](UnitOfWork::transaction):
/// the data layer **commits if the body returns `Ok` and rolls back if it
/// returns `Err`**. Statements the body performs through the data layer join
/// this transaction. To publish an event only after a successful commit (the
/// outbox-relay wake), act on the `Ok` the call returns — no separate
/// `on_commit` hook is needed.
///
/// Scope-based rather than handle-based (`begin`/`commit`) because the DORM
/// executor is closure-scoped (`db::atomic`): a `commit()` called later could
/// not drive a closure that had already returned. Used generically (a handler is
/// generic over its ports), so this trait need not be object-safe.
pub trait UnitOfWork: Send + Sync {
    /// Runs `work` in a transaction, committing on `Ok` and rolling back on `Err`.
    ///
    /// Returns a **boxed `Send` future** (`Pin<Box<dyn Future + Send>>`). The
    /// `Send` bound is part of the contract, so a use case that awaits it stays
    /// `Send` in a generic async context — an HTTP handler drives it directly,
    /// with no `spawn_blocking` offload. Boxing (one allocation per transaction,
    /// negligible against a database round-trip) erases the opaque future type,
    /// which also sidesteps the higher-ranked-lifetime inference limits an
    /// `impl Future` return hits with the generic `work` closure.
    fn transaction<'a, T, F, Fut>(
        &'a self,
        work: F,
    ) -> Pin<Box<dyn Future<Output = PortResult<T>> + Send + 'a>>
    where
        F: FnOnce() -> Fut + Send + 'a,
        Fut: Future<Output = PortResult<T>> + Send + 'a,
        T: Send + 'a;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_renders_as_lowercase_hex() {
        let bytes = [0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let id = Id(bytes);
        // Validate our dependency-free formatter against std's lowercase hex.
        let expected: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(format!("{id:?}"), format!("Id({expected})"));
    }
}
