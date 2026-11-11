//! Wire-schema contribution: how a DTO describes its own JSON shape.
//!
//! A facade descriptor carries parameter and return types as **Rust type name
//! strings** (`"CreateNote"`, `"NoteDto"`), not a structural description — the
//! OpenAPI/proto generators cannot see inside a named type. [`WireSchema`] closes
//! that gap **without reflection**: a DTO contributes a JSON Schema fragment for
//! its own fields, derived at compile time with `#[derive(WireSchema)]`.
//!
//! # How the generator finds a type's schema
//!
//! `#[derive(WireSchema)]` does two things: it implements [`WireSchema`] for the
//! type, and it registers the type's name → schema in a link-time inventory
//! (exactly as `#[seiva::facade]` registers facades, ADR 0001). A generator with
//! only a type *name string* resolves the real schema through
//! [`registered_schema`]; a type that does not derive `WireSchema` is simply
//! absent from the inventory, and the generator falls back to its honest
//! placeholder. The contribution is therefore **opt-in and additive**: no
//! descriptor shape changes, and a type that cannot describe itself still
//! documents honestly.
//!
//! # No hidden magic
//!
//! The derive output has a hand-written equivalent. For
//!
//! ```ignore
//! #[derive(delonix_seiva::WireSchema)]
//! struct CreateNote { title: String }
//! ```
//!
//! it is:
//!
//! ```
//! use delonix_seiva::schema::{object_schema, Field, WireSchema};
//!
//! struct CreateNote { title: String }
//!
//! impl WireSchema for CreateNote {
//!     fn schema() -> serde_json::Value {
//!         object_schema(&[Field {
//!             name: "title",
//!             schema: <String as WireSchema>::schema,
//!             required: true,
//!             flatten: false,
//!         }])
//!     }
//! }
//! // ...plus one `inventory::submit!` registering ("CreateNote", CreateNote::schema).
//! assert_eq!(CreateNote::schema()["properties"]["title"]["type"], "string");
//! ```

use serde_json::{json, Map, Value};

/// A type that can describe its own JSON wire shape as an OpenAPI/JSON Schema
/// fragment, with no reflection. Implemented for the well-known scalars here and
/// derived for DTOs with `#[derive(WireSchema)]`.
pub trait WireSchema {
    /// The JSON Schema fragment for this type (for example
    /// `{"type":"string"}`, or an `object` with the struct's fields).
    fn schema() -> Value;
}

/// One field of a struct schema, as the derive macro emits it: the **wire** name
/// (after serde `rename`/`rename_all`), a function yielding the field type's
/// schema, whether the field is required, and whether it is `#[serde(flatten)]`.
#[derive(Debug, Clone, Copy)]
pub struct Field {
    /// The field's wire name — the name serde serialises it under, so the
    /// published schema matches the JSON on the wire (ADR 0007 fidelity).
    pub name: &'static str,
    /// The field type's schema contribution.
    pub schema: fn() -> Value,
    /// Whether the field is required: `false` for an `Option<_>`, for a
    /// `#[serde(default)]`/`skip_serializing_if`/`skip_deserializing` field, and
    /// irrelevant for a flattened field (its own `required` entries carry over).
    pub required: bool,
    /// Whether this is a `#[serde(flatten)]` field: its (object) schema's
    /// `properties` and `required` are merged into the parent instead of nesting
    /// under `name`, matching how serde flattens the field onto the wire.
    pub flatten: bool,
}

/// Builds an `object` JSON Schema from a struct's fields: a `properties` map
/// keyed by field name and a `required` array of the non-optional field names.
/// `required` is omitted when every field is optional. `additionalProperties` is
/// left unset (the JSON Schema default, `true`) to match serde's lenient
/// deserialisation, which ignores unknown fields.
#[must_use]
pub fn object_schema(fields: &[Field]) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for field in fields {
        if field.flatten {
            // `#[serde(flatten)]`: serde writes the nested object's fields directly
            // onto the parent, with no wrapping key — so merge its `properties` and
            // `required` up, rather than nesting a property under `field.name`.
            let nested = (field.schema)();
            if let Some(props) = nested.get("properties").and_then(Value::as_object) {
                for (key, value) in props {
                    properties.insert(key.clone(), value.clone());
                }
            }
            if let Some(reqs) = nested.get("required").and_then(Value::as_array) {
                required.extend(reqs.iter().cloned());
            }
            continue;
        }
        properties.insert(field.name.to_string(), (field.schema)());
        if field.required {
            required.push(Value::from(field.name));
        }
    }
    let mut object = Map::new();
    object.insert("type".to_string(), Value::from("object"));
    object.insert("properties".to_string(), Value::Object(properties));
    if !required.is_empty() {
        object.insert("required".to_string(), Value::Array(required));
    }
    Value::Object(object)
}

/// The JSON Schema for a C-like, **unit-variant** enum: a `string` constrained to
/// the variant names, matching serde's serialization of a unit variant as its
/// name. `#[derive(WireSchema)]` emits this for such an enum, passing the variant
/// names **after** applying serde `rename`/`rename_all` (so the schema lists the
/// names on the wire, not the Rust ones). Data-carrying enums are rejected by the
/// derive (a faithful `oneOf` is future work).
#[must_use]
pub fn enum_schema(variants: &[&str]) -> Value {
    json!({ "type": "string", "enum": variants })
}

/// A facade DTO's registration in the link-time schema inventory.
///
/// `#[derive(WireSchema)]` emits one `inventory::submit!` of this per type, so
/// every DTO *linked into a binary* is resolvable by name through
/// [`registered_schema`] — the seam an OpenAPI generator uses when it holds only
/// a Rust type-name string.
pub struct SchemaRegistration {
    /// The type's name, as it appears in a facade descriptor (`"CreateNote"`).
    pub name: &'static str,
    /// The type's schema contribution.
    pub schema: fn() -> Value,
}

inventory::collect!(SchemaRegistration);

/// The registered schema for a type named `name`, or `None` when no linked type
/// derived [`WireSchema`] under that name. On a name registered more than once
/// (two DTOs sharing a short name across crates), the first match wins; name
/// collisions are a pre-existing descriptor concern, since descriptors already
/// key types by their short Rust name.
#[must_use]
pub fn registered_schema(name: &str) -> Option<Value> {
    inventory::iter::<SchemaRegistration>
        .into_iter()
        .find(|registration| registration.name == name)
        .map(|registration| (registration.schema)())
}

// --- scalar and std impls ----------------------------------------------------
//
// The mappings mirror `seiva_openapi::schema_for_rust_type` so a field's schema
// is identical whether the generator reaches it by name (the string mapper) or
// by composition (this trait). Owning `WireSchema` here means no orphan-rule
// trouble for the std types.

impl WireSchema for String {
    fn schema() -> Value {
        json!({ "type": "string" })
    }
}

impl WireSchema for str {
    fn schema() -> Value {
        json!({ "type": "string" })
    }
}

impl WireSchema for bool {
    fn schema() -> Value {
        json!({ "type": "boolean" })
    }
}

/// Implements [`WireSchema`] for each integer type as
/// `{"type":"integer","format":"<ty>"}`, matching the string mapper's formats.
macro_rules! integer_schema {
    ($($ty:ty),* $(,)?) => {
        $(impl WireSchema for $ty {
            fn schema() -> Value {
                json!({ "type": "integer", "format": stringify!($ty) })
            }
        })*
    };
}

integer_schema!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);

impl WireSchema for f32 {
    fn schema() -> Value {
        json!({ "type": "number", "format": "float" })
    }
}

impl WireSchema for f64 {
    fn schema() -> Value {
        json!({ "type": "number", "format": "double" })
    }
}

impl WireSchema for () {
    fn schema() -> Value {
        json!({ "type": "null" })
    }
}

/// A `Vec<T>` is a JSON array of `T`. (Unlike the string mapper, this does not
/// special-case `Vec<u8>` as a byte string — a composed field schema describes
/// the element type uniformly.)
impl<T: WireSchema> WireSchema for Vec<T> {
    fn schema() -> Value {
        json!({ "type": "array", "items": T::schema() })
    }
}

/// An `Option<T>` carries `T`'s schema; its *optionality* is expressed by the
/// enclosing struct leaving the field out of `required` (see [`object_schema`]).
impl<T: WireSchema> WireSchema for Option<T> {
    fn schema() -> Value {
        T::schema()
    }
}

/// A reference forwards to the referent, so `&str`, `&CreateNote` and the like
/// resolve without the derive peeling references off field types.
impl<T: WireSchema + ?Sized> WireSchema for &T {
    fn schema() -> Value {
        T::schema()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_match_the_canonical_shapes() {
        assert_eq!(String::schema(), json!({ "type": "string" }));
        assert_eq!(bool::schema(), json!({ "type": "boolean" }));
        assert_eq!(u64::schema(), json!({ "type": "integer", "format": "u64" }));
        assert_eq!(i32::schema(), json!({ "type": "integer", "format": "i32" }));
        assert_eq!(
            f64::schema(),
            json!({ "type": "number", "format": "double" })
        );
        assert_eq!(<()>::schema(), json!({ "type": "null" }));
    }

    #[test]
    fn vec_and_option_and_ref_compose() {
        assert_eq!(
            Vec::<String>::schema(),
            json!({ "type": "array", "items": { "type": "string" } })
        );
        // Option carries the inner schema; the struct expresses optionality.
        assert_eq!(Option::<u32>::schema(), u32::schema());
        assert_eq!(<&str>::schema(), json!({ "type": "string" }));
    }

    #[test]
    fn object_schema_builds_properties_and_required() {
        let schema = object_schema(&[
            Field {
                name: "title",
                schema: <String as WireSchema>::schema,
                required: true,
                flatten: false,
            },
            Field {
                name: "note",
                schema: <String as WireSchema>::schema,
                required: false,
                flatten: false,
            },
        ]);
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["title"], json!({ "type": "string" }));
        assert_eq!(schema["properties"]["note"], json!({ "type": "string" }));
        // Only the non-optional field is required.
        assert_eq!(schema["required"], json!(["title"]));
    }

    #[test]
    fn object_schema_omits_required_when_all_optional() {
        let schema = object_schema(&[Field {
            name: "note",
            schema: <String as WireSchema>::schema,
            required: false,
            flatten: false,
        }]);
        assert!(schema.get("required").is_none());
    }

    // --- Contract fidelity via the derive (ADR 0007/§8) -----------------------
    //
    // These exercise `#[derive(WireSchema)]` end to end — nested DTOs, Option,
    // arrays, bytes and enums — the shapes §8 requires the contract to describe
    // faithfully. `extern crate self as delonix_seiva` lets the derive resolve.

    #[derive(crate::WireSchema)]
    #[allow(dead_code)]
    enum Status {
        Active,
        Suspended,
    }

    #[derive(crate::WireSchema)]
    #[allow(dead_code)]
    struct Address {
        street: String,
        zip: String,
    }

    #[derive(crate::WireSchema)]
    #[allow(dead_code)]
    struct Customer {
        name: String,
        age: u32,
        status: Status,           // nested enum → string enum
        address: Address,         // nested object
        tags: Vec<String>,        // array
        avatar: Vec<u8>,          // bytes (faithful to serde_json: array of integers)
        nickname: Option<String>, // optional
    }

    #[test]
    fn unit_enum_is_a_string_enum() {
        assert_eq!(
            Status::schema(),
            json!({ "type": "string", "enum": ["Active", "Suspended"] })
        );
    }

    #[test]
    fn nested_dto_option_array_and_bytes_are_faithful() {
        let s = Customer::schema();
        assert_eq!(s["type"], "object");
        // Scalars with integer format.
        assert_eq!(
            s["properties"]["age"],
            json!({ "type": "integer", "format": "u32" })
        );
        // Nested enum inlines the string-enum schema.
        assert_eq!(
            s["properties"]["status"],
            json!({ "type": "string", "enum": ["Active", "Suspended"] })
        );
        // Nested object inlines the struct's object schema.
        assert_eq!(s["properties"]["address"]["type"], "object");
        assert_eq!(
            s["properties"]["address"]["properties"]["street"],
            json!({ "type": "string" })
        );
        // Array of strings.
        assert_eq!(
            s["properties"]["tags"],
            json!({ "type": "array", "items": { "type": "string" } })
        );
        // Bytes: serde_json serialises Vec<u8> as an array of integers, so the
        // schema is faithful to the wire (a base64 string would need a serde attr).
        assert_eq!(
            s["properties"]["avatar"],
            json!({ "type": "array", "items": { "type": "integer", "format": "u8" } })
        );
        // Option carries the inner schema; optionality shows only in `required`.
        assert_eq!(s["properties"]["nickname"], json!({ "type": "string" }));
        let required = s["required"].as_array().unwrap();
        for req in ["name", "age", "status", "address", "tags", "avatar"] {
            assert!(required.iter().any(|v| v == req), "{req} must be required");
        }
        assert!(
            !required.iter().any(|v| v == "nickname"),
            "an Option field must not be required"
        );
    }

    // --- serde-attribute fidelity (SC-1 / ADR 0007) --------------------------
    //
    // The published schema must match the JSON serde actually emits. Each test
    // below derives BOTH `serde::Serialize` and `WireSchema` on the same type and
    // asserts the schema's property names equal the real serialised keys — so the
    // two can never drift (the Critical SC-1 was that they silently did).

    /// The property names declared by an `object` schema, sorted.
    fn schema_props(schema: &Value) -> Vec<String> {
        let mut keys: Vec<String> = schema["properties"]
            .as_object()
            .expect("an object schema")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    /// The JSON keys a real instance serialises to, sorted.
    fn json_keys<T: serde::Serialize>(value: &T) -> Vec<String> {
        let json = serde_json::to_value(value).expect("serialises");
        let mut keys: Vec<String> = json
            .as_object()
            .expect("a JSON object")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    #[derive(serde::Serialize, crate::WireSchema)]
    #[serde(rename_all = "camelCase")]
    struct Renamed {
        first_name: String,
        #[serde(rename = "years")]
        age: u32,
    }

    #[test]
    fn rename_all_and_rename_match_the_wire() {
        let schema = Renamed::schema();
        // `rename_all = camelCase` → `firstName`; explicit `rename` → `years`.
        assert_eq!(schema_props(&schema), ["firstName", "years"]);
        assert_eq!(
            schema_props(&schema),
            json_keys(&Renamed {
                first_name: "a".into(),
                age: 1
            }),
            "the schema's property names must equal serde's serialised keys"
        );
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "firstName"));
        assert!(required.iter().any(|v| v == "years"));
    }

    #[derive(serde::Serialize, crate::WireSchema)]
    #[serde(rename_all = "camelCase")]
    struct WithDefaultAndSkip {
        user_name: String,
        #[serde(default)]
        page_size: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(skip)]
        #[allow(dead_code)]
        internal: u64,
    }

    #[test]
    fn default_skip_and_skip_serializing_if_are_reflected() {
        let schema = WithDefaultAndSkip::schema();
        // `skip` field is absent entirely; the rest are camelCased.
        assert_eq!(schema_props(&schema), ["note", "pageSize", "userName"]);
        // `default` and `skip_serializing_if` fields are NOT required; the plain
        // field IS. (`internal` is skipped, so it is neither a property nor required.)
        assert_eq!(schema["required"], json!(["userName"]));
        // A `skip` field must never appear on the wire the schema documents.
        assert!(
            !schema["properties"]
                .as_object()
                .unwrap()
                .contains_key("internal"),
            "a #[serde(skip)] field must not be in the schema"
        );
    }

    #[derive(serde::Serialize, crate::WireSchema)]
    struct Inner {
        a: String,
        b: u32,
    }

    #[derive(serde::Serialize, crate::WireSchema)]
    struct Outer {
        id: u64,
        #[serde(flatten)]
        inner: Inner,
    }

    #[test]
    fn flatten_merges_into_the_parent_matching_the_wire() {
        let schema = Outer::schema();
        // The flattened fields appear at the TOP level, not nested under `inner`.
        assert_eq!(schema_props(&schema), ["a", "b", "id"]);
        assert_eq!(
            schema_props(&schema),
            json_keys(&Outer {
                id: 1,
                inner: Inner {
                    a: "x".into(),
                    b: 2
                }
            }),
            "a flattened struct's fields must appear at the parent level, as serde emits them"
        );
        // The flattened fields carry their own `required`.
        let required = schema["required"].as_array().unwrap();
        for req in ["id", "a", "b"] {
            assert!(required.iter().any(|v| v == req), "{req} must be required");
        }
    }

    #[derive(serde::Serialize, crate::WireSchema)]
    #[serde(rename_all = "SCREAMING_SNAKE_CASE")]
    #[allow(dead_code)]
    enum Mode {
        ReadOnly,
        #[serde(rename = "rw")]
        ReadWrite,
    }

    #[test]
    fn enum_variant_renames_match_the_wire() {
        // `rename_all = SCREAMING_SNAKE_CASE` → `READ_ONLY`; explicit `rename` → `rw`.
        assert_eq!(
            Mode::schema(),
            json!({ "type": "string", "enum": ["READ_ONLY", "rw"] })
        );
        // And that is exactly what serde serialises each variant to.
        assert_eq!(
            serde_json::to_value(Mode::ReadOnly).unwrap(),
            json!("READ_ONLY")
        );
        assert_eq!(serde_json::to_value(Mode::ReadWrite).unwrap(), json!("rw"));
    }
}
