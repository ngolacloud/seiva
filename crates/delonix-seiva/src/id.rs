//! [`WireId`] — a 128-bit identifier that crosses the wire as a JSON **string**.
//!
//! A `u128` serialized as a JSON *number* is unsafe for a large class of clients:
//! JSON's number type is IEEE-754 double in practice, so any integer above 2^53
//! (`Number.MAX_SAFE_INTEGER`) silently loses precision in JavaScript and many
//! other runtimes. Identifiers are exactly where that bites — a mangled id points
//! at the wrong row. `WireId` keeps the full `u128` in memory but serializes as a
//! decimal string, so the wire value is exact everywhere (ADR 0018).
//!
//! Use it for identifier fields on wire DTOs:
//!
//! ```
//! use delonix_seiva::WireId;
//! # use serde::{Serialize, Deserialize};
//! #[derive(Serialize, Deserialize)]
//! struct ProductDto { id: WireId, name: String }
//! ```

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};

use crate::schema::WireSchema;

/// A 128-bit identifier carried on the wire as a decimal **string**, so no client
/// loses precision above 2^53. Wraps a [`u128`]; convert with [`WireId::get`] /
/// `From`/`Into`. See [ADR 0018](../adr/0018-wire-ids-as-strings.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WireId(u128);

impl WireId {
    /// Wraps a raw `u128`.
    pub const fn new(value: u128) -> Self {
        Self(value)
    }

    /// The underlying `u128`.
    pub const fn get(self) -> u128 {
        self.0
    }
}

impl From<u128> for WireId {
    fn from(value: u128) -> Self {
        Self(value)
    }
}

impl From<WireId> for u128 {
    fn from(id: WireId) -> Self {
        id.0
    }
}

impl std::fmt::Display for WireId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for WireId {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u128>().map(Self)
    }
}

impl Serialize for WireId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for WireId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct WireIdVisitor;

        impl Visitor<'_> for WireIdVisitor {
            type Value = WireId;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a 128-bit id as a decimal string (a JSON number is also accepted)")
            }

            // The contract: a decimal string, so the full u128 survives.
            fn visit_str<E: de::Error>(self, value: &str) -> Result<WireId, E> {
                value.parse::<u128>().map(WireId).map_err(de::Error::custom)
            }

            // Lenient: accept a JSON number too (only safe up to u64, which is the
            // most a standard JSON number can carry without precision loss anyway).
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<WireId, E> {
                Ok(WireId(u128::from(value)))
            }

            fn visit_u128<E: de::Error>(self, value: u128) -> Result<WireId, E> {
                Ok(WireId(value))
            }
        }

        deserializer.deserialize_any(WireIdVisitor)
    }
}

impl WireSchema for WireId {
    fn schema() -> Value {
        json!({
            "type": "string",
            "format": "uint128",
            "description": "A 128-bit identifier, carried as a decimal string so it survives clients whose numbers lose precision above 2^53."
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_a_json_string() {
        let big: u128 = 1_794_005_853_720_908_638; // > 2^53
        let json = serde_json::to_string(&WireId::new(big)).unwrap();
        assert_eq!(json, "\"1794005853720908638\"");
    }

    #[test]
    fn round_trips_a_large_id_without_loss() {
        let big = WireId::new(340_282_366_920_938_463_463_374_607_431_768_211_455); // u128::MAX
        let json = serde_json::to_string(&big).unwrap();
        let back: WireId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, big);
    }

    #[test]
    fn deserializes_from_a_string_or_a_number() {
        assert_eq!(
            serde_json::from_str::<WireId>("\"42\"").unwrap(),
            WireId::new(42)
        );
        assert_eq!(
            serde_json::from_str::<WireId>("42").unwrap(),
            WireId::new(42)
        );
    }

    #[test]
    fn schema_is_a_string() {
        assert_eq!(WireId::schema()["type"], "string");
    }

    #[test]
    fn converts_to_and_from_u128() {
        let id = WireId::from(7u128);
        assert_eq!(id.get(), 7);
        assert_eq!(u128::from(id), 7);
        assert_eq!("7".parse::<WireId>().unwrap(), id);
        assert_eq!(id.to_string(), "7");
    }
}
