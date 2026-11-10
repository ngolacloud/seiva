//! Delonix Seiva — gRPC artefacts from a facade descriptor (first cut).
//!
//! This crate is the gRPC companion to `seiva-http`: where that crate serves a
//! module's facades over HTTP, this one turns the same machine-readable
//! [`FacadeDescriptor`] (ADR 0001) into gRPC artefacts. Everything is derived
//! from the descriptor and known at **compile time** — there is no reflection.
//!
//! The core deliverable is [`proto_document`]: it renders a proto3 service
//! document as a `String` from a descriptor. For a facade `Notes` with a
//! `create` method it produces one `service Notes { rpc Create(..) .. }` with a
//! generated request/response `message` per method. It needs nothing but std.
//!
//! A facade descriptor records parameter and return types as **Rust type
//! strings** (`"CreateNote"`, `"String"`, `"&Ctx"`, `"Result<NoteDto, String>"`),
//! not a structural schema. [`proto_document`] therefore maps them pragmatically
//! and *honestly* via [`map_rust_type`]: a type that has a direct proto3 scalar
//! (strings, integers, `bool`, floats, byte slices) becomes that scalar; any
//! other type becomes a `string` placeholder carrying the original Rust type in
//! a `// seiva: <RustType>` trailing comment, so the lossy mapping is auditable
//! rather than hidden. See [`map_rust_type`] for the full table.
//!
//! # Framework context is not a wire field
//!
//! A method may take a framework context argument (`ctx: &Ctx`). That is plumbing,
//! not payload, so [`proto_document`] **skips** any parameter whose Rust type is
//! exactly `&Ctx` or `Ctx` — it never becomes a proto field. See [`is_context`].
//!
//! # tonic wiring
//!
//! The generated `.proto` is the input to tonic's own `tonic-build` codegen,
//! which a project runs in its `build.rs` to get the server trait and message
//! structs. This crate does not re-implement that; it generates the document and,
//! behind the optional, dependency-light `tonic` feature, provides the one piece
//! a generated server still needs: a mapping from a facade error into a gRPC
//! [`Status`](https://docs.rs/tonic/latest/tonic/struct.Status.html) — see the
//! `status` module (compiled only with the `tonic` feature on). A generated
//! service's method implementation is then a thin adapter
//! that dispatches to the facade and translates the result:
//!
//! ```ignore
//! // In a project that ran `tonic_build::compile_protos(..)` on the document
//! // `seiva_grpc::proto_document(&NotesFacade::DESCRIPTOR)` produced.
//! use seiva_grpc::status::status_from_facade_error;
//!
//! #[tonic::async_trait]
//! impl notes_proto::notes_server::Notes for MyNotesService {
//!     async fn create(
//!         &self,
//!         request: tonic::Request<notes_proto::CreateRequest>,
//!     ) -> Result<tonic::Response<notes_proto::CreateResponse>, tonic::Status> {
//!         let wire = request.into_inner();
//!         // Map the wire message onto the facade DTO, call the real facade...
//!         let dto = self
//!             .facade
//!             .create(notes_facade::CreateNote { title: wire.cmd })
//!             .await
//!             .map_err(|e| status_from_facade_error(&e))?; // facade error -> Status
//!         Ok(tonic::Response::new(notes_proto::CreateResponse { value: dto.title }))
//!     }
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use delonix_seiva::contract::{FacadeDescriptor, MethodDescriptor, Param};

/// The proto3 type a Rust type maps to, as decided by [`map_rust_type`].
///
/// Every variant resolves to a valid proto3 scalar through [`proto`](Self::proto);
/// the distinction is only whether the mapping was *structural* (a known scalar)
/// or a *placeholder* (an opaque type carried as a `string` with an audit comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    /// A concrete proto3 scalar the Rust type mapped to directly, e.g. `"string"`,
    /// `"int64"`, `"bool"`, `"bytes"`.
    Scalar(&'static str),
    /// No structural mapping was possible (a struct, enum or alias the descriptor
    /// only knows as a name). The field is emitted as a proto3 `string`, and the
    /// caller should carry the original Rust type in a `// seiva: <type>` comment.
    Placeholder,
}

impl FieldType {
    /// The proto3 scalar to emit. A [`Placeholder`](Self::Placeholder) is `"string"`.
    pub const fn proto(&self) -> &'static str {
        match self {
            FieldType::Scalar(scalar) => scalar,
            FieldType::Placeholder => "string",
        }
    }

    /// Whether this is a [`Placeholder`](Self::Placeholder) (an opaque type carried
    /// as a `string`), which the generator annotates with a `// seiva:` comment.
    pub const fn is_placeholder(&self) -> bool {
        matches!(self, FieldType::Placeholder)
    }
}

/// Maps a canonically rendered Rust type string to a proto3 [`FieldType`].
///
/// The input is a type as the facade descriptor renders it (references and `mut`
/// may lead, generics use `<..>`, one space after a comma). The type is first
/// normalised by stripping any leading `&`, lifetime and `mut`, then matched
/// exactly against this small, deliberately conservative table:
///
/// | Rust (normalised)                     | proto3   |
/// |---------------------------------------|----------|
/// | `String`, `str`                       | `string` |
/// | `bool`                                | `bool`   |
/// | `u8`, `u16`, `u32`                    | `uint32` |
/// | `u64`, `usize`, `u128`                | `uint64` |
/// | `i8`, `i16`, `i32`                    | `int32`  |
/// | `i64`, `isize`, `i128`                | `int64`  |
/// | `f32`                                 | `float`  |
/// | `f64`                                 | `double` |
/// | `Vec<u8>`, `[u8]` (i.e. `&[u8]`)      | `bytes`  |
/// | anything else                         | [`Placeholder`](FieldType::Placeholder) |
///
/// `u128`/`i128` are mapped to the 64-bit proto scalars because proto3 has no
/// 128-bit integer — a lossy but documented choice. The match is **exact** (not a
/// substring test) so a type such as `StringBuilder` does not masquerade as a
/// `String`; unknown types fall through to a placeholder rather than being
/// silently misclassified.
pub fn map_rust_type(rust_type: &str) -> FieldType {
    match normalize(rust_type).as_str() {
        "String" | "str" => FieldType::Scalar("string"),
        "bool" => FieldType::Scalar("bool"),
        "u8" | "u16" | "u32" => FieldType::Scalar("uint32"),
        "u64" | "usize" | "u128" => FieldType::Scalar("uint64"),
        "i8" | "i16" | "i32" => FieldType::Scalar("int32"),
        "i64" | "isize" | "i128" => FieldType::Scalar("int64"),
        "f32" => FieldType::Scalar("float"),
        "f64" => FieldType::Scalar("double"),
        "Vec<u8>" | "[u8]" => FieldType::Scalar("bytes"),
        _ => FieldType::Placeholder,
    }
}

/// Whether a parameter is a framework **context** argument (`&Ctx` or `Ctx`) and
/// therefore not a wire field. The test is exact: only those two spellings count,
/// so a genuine payload type is never dropped by accident.
pub fn is_context(param: &Param) -> bool {
    param.ty == "&Ctx" || param.ty == "Ctx"
}

/// Renders a complete proto3 service document for a facade descriptor.
///
/// The document contains, in order: the `syntax` line, a `package` derived from
/// the facade name and version (`seiva.<name-lowercased>.<version>`), a generated
/// header comment, one `service <Name>` whose RPCs are the facade's methods, and
/// a request/response `message` pair per method.
///
/// Each method `m` yields `rpc <Pascal>(<Pascal>Request) returns (<Pascal>Response);`,
/// where `<Pascal>` is `m.name` in PascalCase (so the snake_case `create_note`
/// becomes `CreateNote`). A guarded method is preceded by a `// requires: <perm>`
/// comment, so the permission the HTTP guard enforces stays visible in the gRPC
/// contract too. The `<Pascal>Request` message has one field per wire parameter —
/// [context parameters](is_context) excluded — numbered from 1, its proto type
/// chosen by [`map_rust_type`]. The `<Pascal>Response` message wraps the method's
/// return type: a `Result<T, E>` is unwrapped to its `T`, a unit return (`()`)
/// yields an empty message, and anything else becomes a single `value` field.
///
/// # Examples
///
/// ```
/// use delonix_seiva::contract::{FacadeDescriptor, MethodDescriptor, Param};
///
/// const DESCRIPTOR: FacadeDescriptor = FacadeDescriptor {
///     name: "Notes",
///     version: "v1",
///     methods: &[MethodDescriptor {
///         name: "create",
///         requires: "notes.create",
///         // A context argument (skipped) and a payload (kept).
///         params: &[
///             Param { name: "ctx", ty: "&Ctx" },
///             Param { name: "title", ty: "String" },
///         ],
///         output: "Result<NoteDto, String>",
///     }],
/// };
///
/// let proto = seiva_grpc::proto_document(&DESCRIPTOR);
/// assert!(proto.contains("service Notes {"));
/// assert!(proto.contains("rpc Create(CreateRequest) returns (CreateResponse);"));
/// // `title: String` became a proto `string` field; `ctx: &Ctx` was skipped.
/// assert!(proto.contains("string title = 1;"));
/// assert!(!proto.contains("ctx"));
/// // The opaque `NoteDto` return is a placeholder carrying its Rust type.
/// assert!(proto.contains("string value = 1; // seiva: NoteDto"));
/// ```
pub fn proto_document(descriptor: &FacadeDescriptor) -> String {
    let mut out = String::new();
    out.push_str("syntax = \"proto3\";\n\n");
    out.push_str(&format!(
        "package seiva.{}.{};\n\n",
        descriptor.name.to_lowercase(),
        descriptor.version
    ));
    out.push_str(&format!(
        "// Generated by seiva-grpc from the `{}` facade descriptor (version {}).\n",
        descriptor.name, descriptor.version
    ));
    out.push_str("// Do not edit by hand; regenerate from the facade contract.\n\n");

    out.push_str(&format!("service {} {{\n", descriptor.name));
    for method in descriptor.methods {
        if method.is_guarded() {
            out.push_str(&format!("  // requires: {}\n", method.requires));
        }
        let rpc = to_pascal_case(method.name);
        out.push_str(&format!(
            "  rpc {rpc}({rpc}Request) returns ({rpc}Response);\n"
        ));
    }
    out.push_str("}\n");

    for method in descriptor.methods {
        let rpc = to_pascal_case(method.name);
        out.push('\n');
        out.push_str(&request_message(&rpc, method));
        out.push('\n');
        out.push_str(&response_message(&rpc, method));
    }
    out
}

/// Renders the `<Rpc>Request` message: one field per wire parameter (context
/// parameters excluded), numbered from 1. No wire parameters yields `{}`.
fn request_message(rpc: &str, method: &MethodDescriptor) -> String {
    let wire: Vec<&Param> = method.params.iter().filter(|p| !is_context(p)).collect();
    if wire.is_empty() {
        return format!("message {rpc}Request {{}}\n");
    }
    let mut out = format!("message {rpc}Request {{\n");
    for (index, param) in wire.iter().enumerate() {
        out.push_str(&field_line(param.name, param.ty, index + 1));
    }
    out.push_str("}\n");
    out
}

/// Renders the `<Rpc>Response` message from the method's return type: a
/// `Result<T, E>` is unwrapped to `T`, a unit (`()`) return yields `{}`, and any
/// other type becomes a single `value` field.
fn response_message(rpc: &str, method: &MethodDescriptor) -> String {
    let ok = unwrap_result(method.output);
    if ok == "()" || ok.is_empty() {
        return format!("message {rpc}Response {{}}\n");
    }
    let mut out = format!("message {rpc}Response {{\n");
    out.push_str(&field_line("value", ok, 1));
    out.push_str("}\n");
    out
}

/// A single proto field line. A placeholder type keeps its Rust spelling in a
/// trailing `// seiva: <type>` comment so the lossy mapping is auditable.
fn field_line(name: &str, rust_type: &str, number: usize) -> String {
    let field = map_rust_type(rust_type);
    if field.is_placeholder() {
        format!(
            "  {} {} = {}; // seiva: {}\n",
            field.proto(),
            name,
            number,
            rust_type
        )
    } else {
        format!("  {} {} = {};\n", field.proto(), name, number)
    }
}

/// Strips any leading `&`, lifetime (`'a`) and `mut` from a rendered Rust type so
/// `&mut String`, `&'a str` and `String` all normalise to the bare type for the
/// [`map_rust_type`] lookup.
fn normalize(ty: &str) -> String {
    let mut rest = ty.trim();
    loop {
        if let Some(stripped) = rest.strip_prefix('&') {
            rest = stripped.trim_start();
            continue;
        }
        if let Some(stripped) = rest.strip_prefix("mut ") {
            rest = stripped.trim_start();
            continue;
        }
        if rest.starts_with('\'') {
            // A lifetime such as `'a`: drop up to the next whitespace.
            if let Some(pos) = rest.find(char::is_whitespace) {
                rest = rest[pos..].trim_start();
                continue;
            }
        }
        break;
    }
    rest.to_string()
}

/// Unwraps a `Result<T, E>` (or `Result<T>`) to its `T`, returning the input
/// unchanged when it is not a `Result`. The `T` is taken as the first top-level
/// generic argument, so a nested generic in `T` (e.g. `Result<Vec<NoteDto>, E>`)
/// is preserved whole.
fn unwrap_result(output: &str) -> &str {
    let trimmed = output.trim();
    match trimmed
        .strip_prefix("Result<")
        .and_then(|inner| inner.strip_suffix('>'))
    {
        Some(inner) => first_generic_arg(inner).trim(),
        None => trimmed,
    }
}

/// The first top-level (depth-0) comma-separated argument of a generic argument
/// list, tracking `<`/`>` nesting so a comma inside a nested generic does not
/// split it.
fn first_generic_arg(inner: &str) -> &str {
    let mut depth = 0usize;
    for (index, ch) in inner.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => return &inner[..index],
            _ => {}
        }
    }
    inner
}

/// Converts a snake_case method name to PascalCase for the proto RPC and message
/// names (`create_note` -> `CreateNote`, `ping` -> `Ping`).
fn to_pascal_case(name: &str) -> String {
    name.split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(feature = "tonic")]
pub mod status {
    //! A facade error mapped onto a gRPC [`Status`], behind the optional `tonic`
    //! feature.
    //!
    //! A facade's error type is opaque to the contract (the descriptor renders it
    //! as a type string, e.g. the `String` in `Result<NoteDto, String>`), so the
    //! mapping works from the error's rendered message. [`facade_error_code`]
    //! classifies that message into a [`Code`] with a small, documented keyword
    //! table, and [`status_from_facade_error`] builds the [`Status`] a generated
    //! service returns. The reverse, [`facade_error_from_status`], recovers the
    //! message a facade would have produced.
    use tonic::{Code, Status};

    /// Classifies a facade error message into a gRPC [`Code`].
    ///
    /// The match is a conservative, case-insensitive keyword scan; anything it
    /// does not recognise is [`Code::Internal`], the safe default for an opaque
    /// server-side failure:
    ///
    /// | message contains (any case)            | [`Code`]                  |
    /// |----------------------------------------|---------------------------|
    /// | `not found`, `missing`                 | [`Code::NotFound`]        |
    /// | `unauthorized`, `forbidden`, `permission` | [`Code::PermissionDenied`] |
    /// | `invalid`, `validation`, `bad request` | [`Code::InvalidArgument`] |
    /// | `conflict`, `already exists`           | [`Code::AlreadyExists`]   |
    /// | `timeout`, `timed out`                 | [`Code::DeadlineExceeded`]|
    /// | anything else                          | [`Code::Internal`]        |
    pub fn facade_error_code(message: &str) -> Code {
        let lower = message.to_ascii_lowercase();
        if lower.contains("not found") || lower.contains("missing") {
            Code::NotFound
        } else if lower.contains("unauthorized")
            || lower.contains("forbidden")
            || lower.contains("permission")
        {
            Code::PermissionDenied
        } else if lower.contains("invalid")
            || lower.contains("validation")
            || lower.contains("bad request")
        {
            Code::InvalidArgument
        } else if lower.contains("conflict") || lower.contains("already exists") {
            Code::AlreadyExists
        } else if lower.contains("timeout") || lower.contains("timed out") {
            Code::DeadlineExceeded
        } else {
            Code::Internal
        }
    }

    /// Builds the [`Status`] a generated gRPC service returns for a facade error:
    /// the [`Code`] from [`facade_error_code`] and the message.
    ///
    /// **Fail-closed on an internal error (V2 / ADR 0019).** A
    /// [`Code::Internal`] is the opaque default for an error the mapping does not
    /// recognise — its message may carry internal detail (a database string, a
    /// path). So, exactly like HTTP's `From<BoxError> for Problem`, the real text
    /// is logged server-side and the client receives an opaque message, unless
    /// `SEIVA_EXPOSE_ERROR_DETAILS` is set (development only; the production config
    /// guard rejects it). A classified (non-`Internal`) code keeps its message: it
    /// is the facade's own contract error, the gRPC twin of a `4xx` problem detail.
    pub fn status_from_facade_error(message: &str) -> Status {
        let code = facade_error_code(message);
        if code == Code::Internal && !seiva_contract::expose_error_details() {
            tracing::error!(error = %message, "internal error mapped to gRPC Internal");
            return Status::new(code, "an internal error occurred");
        }
        Status::new(code, message.to_string())
    }

    /// Recovers the facade error message carried by a [`Status`] — the inverse of
    /// [`status_from_facade_error`] for the message (the [`Code`] is a lossy
    /// classification and is not reconstructed).
    pub fn facade_error_from_status(status: &Status) -> String {
        status.message().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use delonix_seiva::contract::Facade;
    use notes_facade::NotesFacade;

    // A synthetic descriptor that exercises every mapping branch: a context
    // parameter (skipped), a `String` payload (-> `string`), an integer (-> an
    // int scalar), and an opaque return type (-> placeholder).
    const SYNTHETIC: FacadeDescriptor = FacadeDescriptor {
        name: "Accounts",
        version: "v2",
        methods: &[
            MethodDescriptor {
                name: "open_account",
                requires: "accounts.open",
                params: &[
                    Param {
                        name: "ctx",
                        ty: "&Ctx",
                    },
                    Param {
                        name: "name",
                        ty: "String",
                    },
                    Param {
                        name: "balance",
                        ty: "u64",
                    },
                ],
                output: "Result<AccountDto, String>",
            },
            MethodDescriptor {
                name: "ping",
                requires: "",
                params: &[],
                output: "Result<()>",
            },
        ],
    };

    #[test]
    fn renders_service_name_and_rpc_lines_from_the_real_descriptor() {
        let proto = proto_document(&NotesFacade::DESCRIPTOR);
        // The real `notes-facade` facade is `Notes/v1` with one `create` method.
        assert!(proto.contains("service Notes {"), "proto was:\n{proto}");
        assert!(proto.contains("package seiva.notes.v1;"));
        assert!(proto.contains("rpc Create(CreateRequest) returns (CreateResponse);"));
        // `create` is guarded, so the permission is surfaced as a comment.
        assert!(proto.contains("// requires: notes.create"));
    }

    #[test]
    fn maps_the_real_descriptor_payload_and_return_as_placeholders() {
        let proto = proto_document(&NotesFacade::DESCRIPTOR);
        // `cmd: CreateNote` -> opaque placeholder carrying the Rust type.
        assert!(proto.contains("string cmd = 1; // seiva: CreateNote"));
        // `Result<NoteDto, String>` unwraps to `NoteDto` -> placeholder.
        assert!(proto.contains("string value = 1; // seiva: NoteDto"));
    }

    #[test]
    fn a_string_param_maps_to_a_proto_string_field() {
        let proto = proto_document(&SYNTHETIC);
        assert!(proto.contains("string name = 1;"), "proto was:\n{proto}");
    }

    #[test]
    fn an_integer_param_maps_to_an_int_scalar() {
        let proto = proto_document(&SYNTHETIC);
        // `u64` -> `uint64`, numbered after the skipped context parameter.
        assert!(proto.contains("uint64 balance = 2;"), "proto was:\n{proto}");
    }

    #[test]
    fn a_context_param_is_not_a_wire_field() {
        let proto = proto_document(&SYNTHETIC);
        assert!(
            !proto.contains("ctx"),
            "ctx leaked into the proto:\n{proto}"
        );
        // The request message therefore has exactly the two payload fields.
        assert!(proto.contains("message OpenAccountRequest {"));
    }

    #[test]
    fn a_unit_return_yields_an_empty_response_message() {
        let proto = proto_document(&SYNTHETIC);
        assert!(
            proto.contains("message PingResponse {}"),
            "proto was:\n{proto}"
        );
    }

    #[test]
    fn type_mapping_table_is_honest() {
        assert_eq!(map_rust_type("String"), FieldType::Scalar("string"));
        assert_eq!(map_rust_type("&str"), FieldType::Scalar("string"));
        assert_eq!(map_rust_type("bool"), FieldType::Scalar("bool"));
        assert_eq!(map_rust_type("u32"), FieldType::Scalar("uint32"));
        assert_eq!(map_rust_type("i64"), FieldType::Scalar("int64"));
        assert_eq!(map_rust_type("f64"), FieldType::Scalar("double"));
        assert_eq!(map_rust_type("Vec<u8>"), FieldType::Scalar("bytes"));
        assert_eq!(map_rust_type("&[u8]"), FieldType::Scalar("bytes"));
        // A look-alike is NOT misclassified as a `String`.
        assert_eq!(map_rust_type("StringBuilder"), FieldType::Placeholder);
        assert!(map_rust_type("NoteDto").is_placeholder());
    }

    #[cfg(feature = "tonic")]
    #[test]
    fn facade_errors_map_to_grpc_codes() {
        use super::status::{facade_error_code, status_from_facade_error};
        use tonic::Code;

        assert_eq!(facade_error_code("note not found"), Code::NotFound);
        assert_eq!(
            facade_error_code("permission denied for notes.create"),
            Code::PermissionDenied
        );
        assert_eq!(facade_error_code("invalid title"), Code::InvalidArgument);
        assert_eq!(facade_error_code("boom"), Code::Internal);

        let status = status_from_facade_error("note not found");
        assert_eq!(status.code(), Code::NotFound);
        assert_eq!(status.message(), "note not found");
    }

    #[cfg(feature = "tonic")]
    #[test]
    fn an_internal_error_is_opaque_to_the_client() {
        use super::status::status_from_facade_error;
        use tonic::Code;

        // An unrecognised (Internal) error must NOT send its message to the client
        // — parity with HTTP's opaque 500 (V2 / ADR 0019), fail-closed by default
        // (`SEIVA_EXPOSE_ERROR_DETAILS` is off unless a dev sets it). A classified
        // code keeps its message (covered above); only Internal is opaqued.
        let status = status_from_facade_error("db error: relation \"secret\" does not exist");
        assert_eq!(status.code(), Code::Internal);
        assert_eq!(status.message(), "an internal error occurred");
        assert!(
            !status.message().contains("secret"),
            "no internal detail may leak across the gRPC boundary"
        );
    }
}
