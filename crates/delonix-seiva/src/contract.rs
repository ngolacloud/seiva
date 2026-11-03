//! The machine-readable shape of a module's public contract.
//!
//! `#[seiva::facade]` turns a trait into a [`Facade`]: it keeps the trait as the
//! source of truth and generates a zero-sized `<Trait>Facade` type whose
//! [`DESCRIPTOR`](Facade::DESCRIPTOR) lists the facade's name, version and
//! methods, each with the permission it requires. Later phases read this
//! descriptor to generate the `.proto`/gRPC client and to enforce the
//! zero-trust `[[uses]]`/`[[allow]]` contract at build time — no reflection,
//! all known at compile time.
//!
//! # No hidden magic
//!
//! The macro only ADDS the descriptor. Writing it by hand is the exact
//! equivalent of the generated code:
//!
//! ```
//! use delonix_seiva::contract::{Facade, FacadeDescriptor, MethodDescriptor};
//!
//! pub trait Notes {
//!     // async fn create(&self, ..) -> ..;
//! }
//!
//! // What `#[seiva::facade(version = "v1")]` generates:
//! pub struct NotesFacade;
//! impl Facade for NotesFacade {
//!     const DESCRIPTOR: FacadeDescriptor = FacadeDescriptor {
//!         name: "Notes",
//!         version: "v1",
//!         methods: &[MethodDescriptor { name: "create", requires: "notes.create" }],
//!     };
//! }
//! ```

/// A single method a facade exposes, with the permission a caller must hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodDescriptor {
    /// The method name, as written in the trait.
    pub name: &'static str,
    /// The permission declared by the method's `#[requires(..)]`, or `""` when
    /// the method declared none.
    pub requires: &'static str,
}

impl MethodDescriptor {
    /// Whether this method declared a required permission.
    pub const fn is_guarded(&self) -> bool {
        !self.requires.is_empty()
    }
}

/// The full contract of a facade: its name, version and methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FacadeDescriptor {
    /// The facade name (the trait's identifier).
    pub name: &'static str,
    /// The contract version, e.g. `"v1"`. Versions live side by side.
    pub version: &'static str,
    /// Every method the facade exposes, in declaration order.
    pub methods: &'static [MethodDescriptor],
}

impl FacadeDescriptor {
    /// Looks up a method by name.
    pub fn method(&self, name: &str) -> Option<&MethodDescriptor> {
        self.methods.iter().find(|method| method.name == name)
    }
}

/// Implemented by the `<Trait>Facade` type that `#[seiva::facade]` generates.
/// Read [`DESCRIPTOR`](Facade::DESCRIPTOR) to inspect the contract without any
/// macro expansion.
pub trait Facade {
    /// The contract of this facade, known at compile time.
    const DESCRIPTOR: FacadeDescriptor;
}
