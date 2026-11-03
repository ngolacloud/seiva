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

/// A started transaction. The application layer decides where a transaction
/// begins and ends — one per use case — through this port; the data layer only
/// executes it. `on_commit` registers a callback to run after a successful
/// commit (e.g. to wake the outbox relay), never on rollback.
///
/// Bound `Send` but not `Sync`: a transaction handle is moved, used and
/// consumed by one task, never shared by reference across threads — and its
/// `on_commit` hooks (`FnOnce + Send`) are not `Sync`. The [`UnitOfWorkFactory`]
/// that hands it out *is* `Send + Sync`, because that one is a shared dependency.
#[async_trait]
pub trait UnitOfWork: Send {
    /// Commits the transaction and runs the registered `on_commit` hooks.
    async fn commit(self: Box<Self>) -> PortResult<()>;
    /// Rolls the transaction back, discarding the `on_commit` hooks.
    async fn rollback(self: Box<Self>) -> PortResult<()>;
    /// Registers a callback to run after a successful commit.
    fn on_commit(&mut self, hook: Box<dyn FnOnce() + Send + 'static>);
}

/// Opens a [`UnitOfWork`]. Injected into a use case so the handler asks for a
/// transaction without knowing whether it is backed by DORM or an external pool.
#[async_trait]
pub trait UnitOfWorkFactory: Send + Sync {
    async fn begin(&self) -> PortResult<Box<dyn UnitOfWork>>;
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
