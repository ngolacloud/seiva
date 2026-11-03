//! Delonix Seiva — a Rust backend framework for robust, high-criticality
//! systems, with the ease of FastAPI and NestJS, a production- and
//! cluster-ready server, and DORM as the default data layer.
//!
//! This crate is the framework entrypoint a downstream application depends on.
//! It is deliberately thin today: the foundation (repository, contract origin,
//! default bus) is fixed by ADR 0001, the core ports and their test doubles by
//! ADR 0002, and the runtime (HTTP/gRPC, the application-layer command/query
//! dispatch, the worker runtime) is built on top in later phases. The ports an
//! application codes against are re-exported here from [`seiva_ports`] so
//! downstream code has a single import.
//!
//! The core never depends on DORM: the ORM enters only through the
//! `seiva-data-dorm` gateway behind the `dorm` feature (ADR 0001, decision 5).

#![forbid(unsafe_code)]

// Let macro-generated code name this crate by its absolute path (`::delonix_seiva::..`)
// even inside this crate's own tests, mirroring how downstream code sees it.
extern crate self as delonix_seiva;

/// The trait-only boundary the application layer codes against.
pub mod ports {
    pub use seiva_ports::*;
}

pub mod contract;
pub mod lint;
pub mod manifest;
pub mod scaffold;

/// Marks a trait as a module's versioned public contract (ADR 0001). See
/// [`contract`] for what it generates and the hand-written equivalent.
pub use seiva_macros::facade;

/// The framework version, surfaced to the CLI and to `health` endpoints.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether this build links the DORM data layer. Reported by `seiva doctor`
/// so an operator can see at a glance which data layer a binary carries.
pub const DORM_ENABLED: bool = cfg!(feature = "dorm");
