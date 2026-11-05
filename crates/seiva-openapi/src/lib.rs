//! Delonix Seiva — OpenAPI 3.1 generator.
//!
//! This crate turns a module's **facade descriptors** — the machine-readable
//! contracts produced by `#[seiva::facade]` and read through
//! [`FacadeDescriptor`] — into a
//! valid [OpenAPI 3.1](https://spec.openapis.org/oas/v3.1.0) document, with
//! **zero reflection**. It is the real generator that replaces the placeholder
//! `openapi_stub` in `seiva-http`.
//!
//! The entry point is [`openapi_document`]: give it an [`ApiInfo`] and a slice
//! of descriptors, get back a `serde_json::Value` ready to serve from
//! `GET /openapi.json`. Internally the document is a typed model that serialises
//! to the right JSON, so the shape is defined once, in Rust, rather than hand
//! built with `json!`.
//!
//! # Honest, auditable schemas
//!
//! A facade descriptor carries parameter and return **types as Rust type
//! strings** (for example `"CreateNote"`, `"&Ctx"`, `"Result<NoteDto, String>"`),
//! not a structural description. This generator still fabricates nothing: it maps
//! the well-known scalar types to their JSON Schema equivalents, and for a named
//! type it looks the real shape up in the link-time schema inventory
//! ([`delonix_seiva::schema::registered_schema`]) — populated by every DTO that
//! derives [`WireSchema`](delonix_seiva::schema::WireSchema). A named type that
//! did *not* derive `WireSchema` (so is absent from the inventory) falls back to
//! an honest placeholder naming the Rust type:
//!
//! ```json
//! { "type": "string", "x-seiva-rust-type": "CreateNote" }
//! ```
//!
//! An auditor reading the document can see exactly which wire shapes are resolved
//! from a `WireSchema` DTO and which remain placeholders standing in for a named
//! Rust type. See [`schema_for_rust_type`](self#the-type-mapping) for the table.
//!
//! # The path and operation conventions
//!
//! For a facade named `Notes` at version `v1`, the **resource** is the facade
//! name lowercased and pluralised (see [`resource_of`]), and `{version}` is the
//! descriptor's version. Each method maps to one operation:
//!
//! | method name | HTTP          | path                              | body / parameters                         |
//! |-------------|---------------|-----------------------------------|-------------------------------------------|
//! | `create`    | `POST`        | `/{version}/{resource}`           | request body from the non-context params  |
//! | `get`       | `GET`         | `/{version}/{resource}/{id}`      | first non-context param → path, rest → query |
//! | `list`      | `GET`         | `/{version}/{resource}`           | non-context params → query                |
//! | *(other)*   | `POST`        | `/{version}/{resource}/{method}`  | request body from the non-context params  |
//!
//! `operationId` is `<facade-lowercased>_<method>` (for example
//! `notes_create`). A `create` operation answers `201`; every other operation
//! answers `200`.
//!
//! ## Context parameters are not wire parameters
//!
//! A parameter typed exactly `&Ctx` or `Ctx` is the framework request context,
//! injected by the transport — it is never a wire field. Such parameters are
//! skipped when building request bodies, path parameters and query parameters.
//!
//! ## Request bodies are the parameter DTO directly
//!
//! A write method takes at most one body parameter (the generated
//! `#[seiva::controller]` enforces this and deserialises the body *as* that
//! parameter's DTO through `seiva_http::Valid<T>`). The request body schema is
//! therefore that parameter's [schema](self#the-type-mapping) directly — not an
//! object keyed by the parameter name, which would mis-document the wire
//! contract. A write with no body parameter carries no body. (A hand-built
//! descriptor declaring more than one body parameter — impossible through the
//! controller — falls back to an object keyed by parameter name.)
//!
//! ## Responses unwrap one `Result` level
//!
//! A return type is parsed for a single leading `Result<T, E>` / `Result<T>`
//! layer; the success payload `T` is schema-ised for the `2xx` response. So
//! `Result<NoteDto, String>` documents a response body schema for `NoteDto`
//! (as the placeholder type), and `Result<()>` documents a `null` body.
//!
//! ## Security
//!
//! A guarded method — one whose descriptor
//! [`is_guarded()`](delonix_seiva::contract::MethodDescriptor::is_guarded) — gets
//! a `security` requirement referencing the document's bearer
//! `securityScheme` (`bearerAuth`), and carries the required permission in an
//! `x-seiva-requires` operation extension. The `securityScheme` is declared
//! under `components` only when at least one method is guarded.
//!
//! # Validation
//!
//! The crate keeps its dependency surface to the contract crate plus serde, so
//! it does **not** pull in an OpenAPI meta-schema validator. The tests assert
//! the document's structure directly (the `openapi` version, the presence of
//! operations under their expected path and verb, the extensions and the
//! schemas). This is called out explicitly rather than claimed as full
//! meta-schema validation.
//!
//! # Example
//!
//! ```
//! use seiva_openapi::{openapi_document, ApiInfo};
//! use notes_facade::NotesFacade;
//! use delonix_seiva::contract::Facade;
//!
//! let doc = openapi_document(
//!     ApiInfo { title: "Notes API".into(), version: "1.0.0".into() },
//!     &[NotesFacade::DESCRIPTOR],
//! );
//! assert_eq!(doc["openapi"], "3.1.0");
//! assert!(doc["paths"]["/v1/notes"]["post"].is_object());
//! ```

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{json, Value};

use delonix_seiva::contract::{FacadeDescriptor, MethodDescriptor, Param};

/// The top-level `info` a generated document carries: the service title and
/// version shown to clients. Maps to OpenAPI's required `info.title` and
/// `info.version`.
#[derive(Debug, Clone)]
pub struct ApiInfo {
    /// The human-readable service title (`info.title`).
    pub title: String,
    /// The service version string (`info.version`) — the API's version, not the
    /// OpenAPI specification version.
    pub version: String,
}

/// Generates an OpenAPI 3.1 document from an [`ApiInfo`] and one or more facade
/// descriptors, returning it as a `serde_json::Value` ready to serve.
///
/// Every method of every descriptor becomes one operation, following the
/// conventions documented at the [crate root](crate). Facades are processed in
/// the given order; within a facade, methods keep their declaration order. When
/// two operations share a path (for example a `create` `POST` and a `list`
/// `GET` on the same resource) they merge into one path item.
///
/// # Example
///
/// ```
/// use seiva_openapi::{openapi_document, ApiInfo};
/// use notes_facade::NotesFacade;
/// use delonix_seiva::contract::Facade;
///
/// let doc = openapi_document(
///     ApiInfo { title: "Notes API".into(), version: "1.0.0".into() },
///     &[NotesFacade::DESCRIPTOR],
/// );
/// assert_eq!(doc["info"]["title"], "Notes API");
/// ```
pub fn openapi_document(info: ApiInfo, facades: &[FacadeDescriptor]) -> Value {
    let mut paths: BTreeMap<String, PathItem> = BTreeMap::new();
    let mut any_guarded = false;

    for facade in facades {
        let resource = resource_of(facade.name);
        for method in facade.methods {
            if method.is_guarded() {
                any_guarded = true;
            }
            let route = route_for(facade, &resource, method);
            let operation = operation_for(facade, method, &route);
            let item = paths.entry(route.path).or_default();
            match route.verb {
                Verb::Get => item.get = Some(operation),
                Verb::Post => item.post = Some(operation),
            }
        }
    }

    let components = any_guarded.then(Components::bearer);

    let document = OpenApiDocument {
        openapi: "3.1.0",
        info: DocInfo {
            title: info.title,
            version: info.version,
        },
        paths,
        components,
    };

    serde_json::to_value(&document).expect("the typed OpenAPI model always serialises to JSON")
}

/// The resource segment for a facade: its name lowercased, then pluralised by
/// appending `s` unless it already ends in `s`.
///
/// The rule is deliberately naive — it is a path convention, not a linguistics
/// engine. `Notes` → `notes` (already ends in `s`); `Account` → `accounts`;
/// `Invoice` → `invoices`.
pub fn resource_of(facade_name: &str) -> String {
    let lower = facade_name.to_ascii_lowercase();
    if lower.ends_with('s') {
        lower
    } else {
        format!("{lower}s")
    }
}

/// Maps a Rust type string to a JSON Schema fragment, honestly.
///
/// The well-known scalar types map to their JSON Schema equivalents:
///
/// | Rust type                                            | JSON Schema                               |
/// |------------------------------------------------------|-------------------------------------------|
/// | `String`, `&str`, `str`, `&String`                   | `{"type":"string"}`                       |
/// | `bool`                                               | `{"type":"boolean"}`                      |
/// | `i8`…`i128`, `isize`, `u8`…`u128`, `usize`           | `{"type":"integer","format":"<ty>"}`      |
/// | `f32`                                                | `{"type":"number","format":"float"}`      |
/// | `f64`                                                | `{"type":"number","format":"double"}`     |
/// | `Vec<u8>`                                            | `{"type":"string","format":"byte"}`       |
/// | `Vec<T>`                                             | `{"type":"array","items":<schema of T>}`  |
/// | `()`                                                 | `{"type":"null"}`                         |
/// | *(a `WireSchema` DTO, by name)*                      | its registered object schema              |
/// | *(anything else)*                                    | `{"type":"string","x-seiva-rust-type":"<ty>"}` |
///
/// A named type is resolved from the link-time schema inventory when it derived
/// [`WireSchema`](delonix_seiva::schema::WireSchema); otherwise the final row is
/// the honest placeholder — the generator records the Rust type name rather than
/// fabricating a shape.
pub fn schema_for_rust_type(ty: &str) -> Value {
    let ty = ty.trim();
    match ty {
        "String" | "&str" | "str" | "&String" => json!({ "type": "string" }),
        "bool" => json!({ "type": "boolean" }),
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128"
        | "usize" => json!({ "type": "integer", "format": ty }),
        "f32" => json!({ "type": "number", "format": "float" }),
        "f64" => json!({ "type": "number", "format": "double" }),
        "()" => json!({ "type": "null" }),
        "Vec<u8>" => json!({ "type": "string", "format": "byte" }),
        _ => {
            if let Some(inner) = ty.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
                json!({ "type": "array", "items": schema_for_rust_type(inner) })
            } else if let Some(real) = delonix_seiva::schema::registered_schema(ty) {
                // A DTO that derived `WireSchema` contributed its real shape to
                // the link-time inventory; use it instead of the placeholder.
                real
            } else {
                json!({ "type": "string", "x-seiva-rust-type": ty })
            }
        }
    }
}

/// Whether a parameter is the framework request context (typed exactly `&Ctx`
/// or `Ctx`) and so must not appear on the wire.
fn is_context_param(param: &Param) -> bool {
    matches!(param.ty.trim(), "Ctx" | "&Ctx")
}

/// The success payload type of a return string: one leading `Result<T, E>` /
/// `Result<T>` layer is unwrapped to `T`; anything else is returned unchanged.
fn success_payload(output: &str) -> &str {
    let output = output.trim();
    match output
        .strip_prefix("Result<")
        .and_then(|rest| rest.strip_suffix('>'))
    {
        Some(args) => first_type_argument(args),
        None => output,
    }
}

/// The first top-level type argument of a (comma-separated) generic argument
/// list, respecting nested `<...>`. `"NoteDto, String"` → `"NoteDto"`;
/// `"Vec<u8>, Error"` → `"Vec<u8>"`; `"NoteDto"` → `"NoteDto"`.
fn first_type_argument(args: &str) -> &str {
    let mut depth = 0usize;
    for (index, ch) in args.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => return args[..index].trim(),
            _ => {}
        }
    }
    args.trim()
}

/// The HTTP verb an operation uses.
#[derive(Debug, Clone, Copy)]
enum Verb {
    Get,
    Post,
}

/// A method's resolved routing: the path template and the verb.
struct Route {
    path: String,
    verb: Verb,
    /// The success status code (`"201"` for `create`, else `"200"`).
    status: &'static str,
    /// For a `get`, the name of the path parameter (the first non-context
    /// parameter), if any.
    path_param: Option<&'static str>,
}

/// Resolves a method to its route per the crate's conventions.
fn route_for(facade: &FacadeDescriptor, resource: &str, method: &MethodDescriptor) -> Route {
    let base = format!("/{}/{}", facade.version, resource);
    match method.name {
        "create" => Route {
            path: base,
            verb: Verb::Post,
            status: "201",
            path_param: None,
        },
        "get" => {
            let path_param = method
                .params
                .iter()
                .find(|param| !is_context_param(param))
                .map(|param| param.name);
            let path = match path_param {
                Some(name) => format!("{base}/{{{name}}}"),
                None => base,
            };
            Route {
                path,
                verb: Verb::Get,
                status: "200",
                path_param,
            }
        }
        "list" => Route {
            path: base,
            verb: Verb::Get,
            status: "200",
            path_param: None,
        },
        other => Route {
            path: format!("{base}/{other}"),
            verb: Verb::Post,
            status: "200",
            path_param: None,
        },
    }
}

/// Builds the operation object for a method under its resolved route.
fn operation_for(facade: &FacadeDescriptor, method: &MethodDescriptor, route: &Route) -> Operation {
    // Wire parameters: every declared parameter that is not the request context.
    let wire_params: Vec<&Param> = method
        .params
        .iter()
        .filter(|param| !is_context_param(param))
        .collect();

    let mut parameters = Vec::new();
    let mut request_body = None;

    match route.verb {
        Verb::Get => {
            // GET carries no body: each wire parameter is a path or query field.
            for param in &wire_params {
                let in_path = route.path_param == Some(param.name);
                parameters.push(Parameter {
                    name: param.name.to_string(),
                    location: if in_path { "path" } else { "query" }.to_string(),
                    required: in_path,
                    schema: schema_for_rust_type(param.ty),
                });
            }
        }
        Verb::Post => {
            // The generated `#[controller]` deserialises a write body AS the single
            // parameter's DTO directly (`seiva_http::Valid<T>`), and rejects more
            // than one body parameter. So the body schema IS that DTO's schema — an
            // object keyed by parameter name would mis-document the wire contract.
            match wire_params.as_slice() {
                [] => {}
                [only] => {
                    request_body = Some(RequestBody {
                        required: true,
                        content: media("application/json", schema_for_rust_type(only.ty)),
                    });
                }
                many => {
                    // Unreachable through `#[controller]` (it forbids >1 body
                    // parameter), but a hand-built descriptor could declare it;
                    // document it honestly as an object keyed by parameter name.
                    let mut properties = serde_json::Map::new();
                    let mut required = Vec::new();
                    for param in many {
                        properties.insert(param.name.to_string(), schema_for_rust_type(param.ty));
                        required.push(Value::from(param.name));
                    }
                    let schema = json!({
                        "type": "object",
                        "properties": Value::Object(properties),
                        "required": Value::Array(required),
                    });
                    request_body = Some(RequestBody {
                        required: true,
                        content: media("application/json", schema),
                    });
                }
            }
        }
    }

    let response_schema = schema_for_rust_type(success_payload(method.output));
    let mut responses = BTreeMap::new();
    responses.insert(
        route.status.to_string(),
        ResponseObject {
            description: if route.status == "201" {
                "Created"
            } else {
                "OK"
            }
            .to_string(),
            content: Some(media("application/json", response_schema)),
        },
    );

    let (security, requires) = if method.is_guarded() {
        (
            vec![BTreeMap::from([("bearerAuth".to_string(), Vec::new())])],
            Some(method.requires.to_string()),
        )
    } else {
        (Vec::new(), None)
    };

    Operation {
        operation_id: format!("{}_{}", facade.name.to_ascii_lowercase(), method.name),
        parameters,
        request_body,
        responses,
        security,
        requires,
    }
}

/// A single-entry `content` map for one media type and schema.
fn media(media_type: &str, schema: Value) -> BTreeMap<String, MediaType> {
    BTreeMap::from([(media_type.to_string(), MediaType { schema })])
}

// ---------------------------------------------------------------------------
// The typed document model. These types are internal: they exist to serialise
// to the right JSON through `serde`, and the public surface is `Value`.
// ---------------------------------------------------------------------------

/// The root OpenAPI document.
#[derive(Serialize)]
struct OpenApiDocument {
    openapi: &'static str,
    info: DocInfo,
    paths: BTreeMap<String, PathItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    components: Option<Components>,
}

/// The `info` object.
#[derive(Serialize)]
struct DocInfo {
    title: String,
    version: String,
}

/// A path item, holding the operations keyed by verb on one path.
#[derive(Serialize, Default)]
struct PathItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    get: Option<Operation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    post: Option<Operation>,
}

/// An operation object.
#[derive(Serialize)]
struct Operation {
    #[serde(rename = "operationId")]
    operation_id: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    parameters: Vec<Parameter>,
    #[serde(rename = "requestBody", skip_serializing_if = "Option::is_none")]
    request_body: Option<RequestBody>,
    responses: BTreeMap<String, ResponseObject>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    security: Vec<BTreeMap<String, Vec<String>>>,
    #[serde(rename = "x-seiva-requires", skip_serializing_if = "Option::is_none")]
    requires: Option<String>,
}

/// A path or query parameter.
#[derive(Serialize)]
struct Parameter {
    name: String,
    #[serde(rename = "in")]
    location: String,
    required: bool,
    schema: Value,
}

/// A request body object.
#[derive(Serialize)]
struct RequestBody {
    required: bool,
    content: BTreeMap<String, MediaType>,
}

/// A media type object (carries a schema).
#[derive(Serialize)]
struct MediaType {
    schema: Value,
}

/// A response object.
#[derive(Serialize)]
struct ResponseObject {
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<BTreeMap<String, MediaType>>,
}

/// The `components` object; here, only the security schemes.
#[derive(Serialize)]
struct Components {
    #[serde(rename = "securitySchemes")]
    security_schemes: BTreeMap<String, Value>,
}

impl Components {
    /// Components declaring a single HTTP bearer security scheme named
    /// `bearerAuth`.
    fn bearer() -> Self {
        Components {
            security_schemes: BTreeMap::from([(
                "bearerAuth".to_string(),
                json!({ "type": "http", "scheme": "bearer" }),
            )]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use delonix_seiva::contract::Facade;
    use notes_facade::NotesFacade;

    /// A synthetic descriptor exercising a `&Ctx` context parameter and a
    /// `get`, which the real `notes-facade` does not declare.
    const CTX_FACADE: FacadeDescriptor = FacadeDescriptor {
        name: "Accounts",
        version: "v2",
        methods: &[
            MethodDescriptor {
                name: "create",
                requires: "accounts.create",
                params: &[
                    Param {
                        name: "ctx",
                        ty: "&Ctx",
                    },
                    Param {
                        name: "cmd",
                        ty: "CreateAccount",
                    },
                ],
                output: "Result<AccountDto, String>",
            },
            MethodDescriptor {
                name: "get",
                requires: "",
                params: &[
                    Param {
                        name: "ctx",
                        ty: "&Ctx",
                    },
                    Param {
                        name: "id",
                        ty: "u64",
                    },
                ],
                output: "Result<AccountDto>",
            },
        ],
    };

    fn notes_doc() -> Value {
        openapi_document(
            ApiInfo {
                title: "Notes API".into(),
                version: "1.0.0".into(),
            },
            &[NotesFacade::DESCRIPTOR],
        )
    }

    #[test]
    fn document_declares_openapi_3_1_and_info() {
        let doc = notes_doc();
        assert_eq!(doc["openapi"], "3.1.0");
        assert_eq!(doc["info"]["title"], "Notes API");
        assert_eq!(doc["info"]["version"], "1.0.0");
    }

    #[test]
    fn create_maps_to_post_under_the_versioned_resource_path() {
        let doc = notes_doc();
        let create = &doc["paths"]["/v1/notes"]["post"];
        assert!(create.is_object(), "create operation must exist");
        assert_eq!(create["operationId"], "notes_create");
    }

    #[test]
    fn guarded_create_declares_requires_extension_and_security() {
        let doc = notes_doc();
        let create = &doc["paths"]["/v1/notes"]["post"];
        assert_eq!(create["x-seiva-requires"], "notes.create");
        assert_eq!(create["security"][0]["bearerAuth"], json!([]));
        // The bearer scheme is declared in components.
        assert_eq!(
            doc["components"]["securitySchemes"]["bearerAuth"]["scheme"],
            "bearer"
        );
    }

    #[test]
    fn create_request_body_is_the_dto_schema_directly() {
        let doc = notes_doc();
        let schema = &doc["paths"]["/v1/notes"]["post"]["requestBody"]["content"]
            ["application/json"]["schema"];
        // The body is `CreateNote` itself (what `Valid<CreateNote>` deserialises),
        // NOT an object keyed by the `cmd` parameter name. `CreateNote` derived
        // `WireSchema`, so its real fields are resolved from the inventory.
        assert_eq!(schema["type"], "object");
        assert!(
            schema
                .get("properties")
                .and_then(|p| p.get("cmd"))
                .is_none(),
            "the body must not be wrapped under the `cmd` parameter name"
        );
        assert_eq!(schema["properties"]["title"], json!({ "type": "string" }));
        assert_eq!(schema["required"], json!(["title"]));
    }

    #[test]
    fn return_type_resolves_the_dto_schema_from_the_inventory() {
        let doc = notes_doc();
        let schema = &doc["paths"]["/v1/notes"]["post"]["responses"]["201"]["content"]
            ["application/json"]["schema"];
        // `Result<NoteDto, String>` → `NoteDto`, resolved to its real shape.
        assert_eq!(
            schema,
            &json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "format": "u128" },
                    "title": { "type": "string" },
                },
                "required": ["id", "title"],
            })
        );
    }

    #[test]
    fn a_context_parameter_produces_no_wire_field() {
        let doc = openapi_document(
            ApiInfo {
                title: "Accounts API".into(),
                version: "1.0.0".into(),
            },
            &[CTX_FACADE],
        );
        // create: the only wire body parameter is `cmd` (`ctx: &Ctx` is skipped),
        // so the body is `CreateAccount`'s schema directly. `CreateAccount` is a
        // synthetic type with no `WireSchema` impl, so it stays a placeholder —
        // which also proves `ctx` was dropped (otherwise the body would be a
        // two-property object, not a bare placeholder).
        let body = &doc["paths"]["/v2/accounts"]["post"]["requestBody"]["content"]
            ["application/json"]["schema"];
        assert_eq!(
            body,
            &json!({ "type": "string", "x-seiva-rust-type": "CreateAccount" })
        );

        // get: `id: u64` is the path parameter; `ctx` is not a parameter at all.
        let get = &doc["paths"]["/v2/accounts/{id}"]["get"];
        let params = get["parameters"].as_array().unwrap();
        assert_eq!(params.len(), 1, "only `id` is a wire parameter");
        assert_eq!(params[0]["name"], "id");
        assert_eq!(params[0]["in"], "path");
        assert_eq!(params[0]["required"], true);
        assert_eq!(
            params[0]["schema"],
            json!({ "type": "integer", "format": "u64" })
        );
    }

    #[test]
    fn an_unguarded_method_has_no_security_or_requires() {
        let doc = openapi_document(
            ApiInfo {
                title: "Accounts API".into(),
                version: "1.0.0".into(),
            },
            &[CTX_FACADE],
        );
        let get = &doc["paths"]["/v2/accounts/{id}"]["get"];
        assert!(get.get("security").is_none());
        assert!(get.get("x-seiva-requires").is_none());
    }

    #[test]
    fn result_unwrapping_takes_the_success_payload() {
        assert_eq!(success_payload("Result<NoteDto, String>"), "NoteDto");
        assert_eq!(success_payload("Result<NoteDto>"), "NoteDto");
        assert_eq!(success_payload("Result<Vec<u8>, Error>"), "Vec<u8>");
        assert_eq!(success_payload("NoteDto"), "NoteDto");
        assert_eq!(success_payload("Result<()>"), "()");
    }

    #[test]
    fn the_type_table_maps_the_well_known_scalars() {
        assert_eq!(schema_for_rust_type("String"), json!({ "type": "string" }));
        assert_eq!(schema_for_rust_type("bool"), json!({ "type": "boolean" }));
        assert_eq!(
            schema_for_rust_type("i32"),
            json!({ "type": "integer", "format": "i32" })
        );
        assert_eq!(
            schema_for_rust_type("Vec<u8>"),
            json!({ "type": "string", "format": "byte" })
        );
        assert_eq!(
            schema_for_rust_type("Vec<String>"),
            json!({ "type": "array", "items": { "type": "string" } })
        );
        assert_eq!(schema_for_rust_type("()"), json!({ "type": "null" }));
        // An unknown named type (no `WireSchema` impl linked) stays a placeholder.
        assert_eq!(
            schema_for_rust_type("TotallyUnknownType"),
            json!({ "type": "string", "x-seiva-rust-type": "TotallyUnknownType" })
        );
    }

    #[test]
    fn a_wire_schema_dto_resolves_to_its_real_shape_not_a_placeholder() {
        // `CreateNote` derived `WireSchema` (notes-facade is a dev-dependency, so
        // it is linked here), so the generator resolves its fields by name rather
        // than emitting the `x-seiva-rust-type` placeholder.
        assert_eq!(
            schema_for_rust_type("CreateNote"),
            json!({
                "type": "object",
                "properties": { "title": { "type": "string" } },
                "required": ["title"],
            })
        );
    }

    #[test]
    fn resource_pluralisation_follows_the_naive_rule() {
        assert_eq!(resource_of("Notes"), "notes");
        assert_eq!(resource_of("Account"), "accounts");
        assert_eq!(resource_of("Invoice"), "invoices");
    }
}
