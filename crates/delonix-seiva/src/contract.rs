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
//! use delonix_seiva::contract::{Facade, FacadeDescriptor, MethodDescriptor, Param};
//!
//! pub trait Notes {
//!     // async fn create(&self, cmd: CreateNote) -> Result<NoteDto, String>;
//! }
//!
//! // What `#[seiva::facade(version = "v1")]` generates:
//! pub struct NotesFacade;
//! impl Facade for NotesFacade {
//!     const DESCRIPTOR: FacadeDescriptor = FacadeDescriptor {
//!         name: "Notes",
//!         version: "v1",
//!         methods: &[MethodDescriptor {
//!             name: "create",
//!             requires: "notes.create",
//!             params: &[Param { name: "cmd", ty: "CreateNote" }],
//!             output: "Result<NoteDto, String>",
//!         }],
//!     };
//! }
//! ```

/// A single parameter of a facade method: its name and its Rust type, rendered
/// from the trait's tokens as it was written (e.g. `"CreateNote"`, `"&Ctx"`,
/// `"Result<NoteDto, String>"`). The receiver `&self` is not a parameter and
/// never appears. A framework context argument (e.g. a `&Ctx`) is captured like
/// any other; a transport generator decides which parameters map to the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param {
    /// The parameter name, as written in the trait.
    pub name: &'static str,
    /// The parameter's Rust type, rendered canonically from the trait's tokens
    /// (spaces against punctuation dropped; one space after a comma).
    pub ty: &'static str,
}

/// A single method a facade exposes, with the permission a caller must hold and
/// its full signature (parameters and return type), so a generator can build a
/// `.proto` service, an HTTP route or an OpenAPI operation with no reflection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodDescriptor {
    /// The method name, as written in the trait.
    pub name: &'static str,
    /// The permission declared by the method's `#[requires(..)]`, or `""` when
    /// the method declared none.
    pub requires: &'static str,
    /// The method's parameters (the `&self` receiver excluded), in declaration
    /// order, each with its name and Rust type.
    pub params: &'static [Param],
    /// The method's return type, rendered canonically from the trait's tokens
    /// (`"()"` when the method returns nothing).
    pub output: &'static str,
}

impl MethodDescriptor {
    /// Whether this method declared a required permission.
    pub const fn is_guarded(&self) -> bool {
        !self.requires.is_empty()
    }

    /// Looks up a parameter by name.
    pub fn param(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|param| param.name == name)
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

/// A facade's registration in the link-time inventory.
///
/// `#[seiva::facade]` emits one `inventory::submit!` of this per facade, so
/// every facade *linked into a binary* is discoverable through
/// [`registered_facades`] without a hand-maintained list. This is how the
/// runtime's `/openapi.json` lists every mounted facade (OWASP API9: Improper
/// Inventory Management) rather than a single hard-coded one.
pub struct FacadeRegistration {
    /// The registered facade's contract.
    pub descriptor: FacadeDescriptor,
}

inventory::collect!(FacadeRegistration);

/// Every facade registered by `#[seiva::facade]` and linked into the current
/// binary, sorted by `(name, version)` so the order is deterministic regardless
/// of link order.
///
/// A standalone tool (the `seiva` CLI) sees only the facades compiled into it,
/// not a user project's own crates; the *runtime* binary that mounts the modules
/// sees them all, so its generated document is the complete one.
#[must_use]
pub fn registered_facades() -> Vec<FacadeDescriptor> {
    let mut facades: Vec<FacadeDescriptor> = inventory::iter::<FacadeRegistration>
        .into_iter()
        .map(|registration| registration.descriptor)
        .collect();
    facades.sort_by(|a, b| (a.name, a.version).cmp(&(b.name, b.version)));
    facades
}

/// Implemented by the `<Trait>Facade` type that `#[seiva::facade]` generates.
/// Read [`DESCRIPTOR`](Facade::DESCRIPTOR) to inspect the contract without any
/// macro expansion.
pub trait Facade {
    /// The contract of this facade, known at compile time.
    const DESCRIPTOR: FacadeDescriptor;
}
