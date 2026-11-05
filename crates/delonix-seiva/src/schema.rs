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

/// One field of a struct schema, as the derive macro emits it: the wire name, a
/// function yielding the field type's schema, and whether the field is required
/// (every field except an `Option<_>` is).
#[derive(Debug, Clone, Copy)]
pub struct Field {
    /// The field's wire name.
    pub name: &'static str,
    /// The field type's schema contribution.
    pub schema: fn() -> Value,
    /// Whether the field is required (false for `Option<_>`).
    pub required: bool,
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
            },
            Field {
                name: "note",
                schema: <String as WireSchema>::schema,
                required: false,
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
        }]);
        assert!(schema.get("required").is_none());
    }
}
