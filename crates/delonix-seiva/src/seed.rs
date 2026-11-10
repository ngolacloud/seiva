//! Per-module seed and demo data as CSV (ADR 0022).
//!
//! A module ships two optional CSV folders, in the Odoo spirit:
//!
//! - `modules/<name>/seed/*.csv` — **reference data**, always loaded on boot,
//!   through the data layer with an **idempotent upsert** (no domain events). The
//!   row's stable textual key (the `id` column) is turned into a deterministic
//!   128-bit id by [`external_id`], so re-running the seed overwrites rather than
//!   duplicating, and one module's row can be referenced by another by the same key.
//! - `modules/<name>/demo/*.csv` — **demonstration data**, loaded **only in dev**
//!   (and staging), **through the module facade** so a create exercises the domain
//!   invariants (and publishes the domain event, where the facade does). It is the
//!   fixture a developer boots a service with to see it working end to end.
//!
//! Each CSV is embedded into the binary with `include_str!` and declared with
//! [`register_seed!`](crate::register_seed), mirroring the admin registry
//! ([`admin`](crate::admin)): the registration is link-time metadata discoverable
//! through [`registered_seeds`], so a fitness function and the `seiva db seed` CLI
//! can see what a binary carries. The *application* of the data (upsert / facade
//! call) is wired in the composition root, the only place that holds the concrete
//! adapters — exactly like the module router merges.
//!
//! The CSV parser ([`parse_csv`]) is a dependency-free RFC 4180 subset (a header
//! row, quoted fields with `""` escaping, `#` comment lines), so the default
//! DORM-free build takes on no new supply-chain surface.
//!
//! ```
//! use delonix_seiva::seed::{external_id, parse_csv};
//!
//! // A stable textual key maps to the same id every run.
//! assert_eq!(external_id("catalog.product", "apple"), external_id("catalog.product", "apple"));
//!
//! let rows = parse_csv("id,title\napple,Apple\npear,\"Pear, green\"\n");
//! assert_eq!(rows[0]["title"], "Apple");
//! assert_eq!(rows[1]["title"], "Pear, green");
//! ```

use seiva_ports::Id;
use std::collections::BTreeMap;

/// A module's seed or demo CSV source, declared with
/// [`register_seed!`](crate::register_seed). All fields are `&'static`, so a
/// registration is a `const` expression `inventory::submit!` can take at item scope
/// (the CSV body is an `include_str!` literal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedSource {
    /// The owning module/namespace, e.g. `"catalog"`.
    pub module: &'static str,
    /// The resource within the module, e.g. `"product"`. Matches the module's
    /// route/resource segment so a reviewer can pair it with `/openapi.json`.
    pub resource: &'static str,
    /// The CSV body, embedded via `include_str!`.
    pub csv: &'static str,
    /// `true` for a `demo/` file (dev-only, via facade); `false` for a `seed/`
    /// file (always, idempotent upsert).
    pub demo: bool,
}

impl SeedSource {
    /// The stable id `{module}/{resource}` (plus `#demo` for a demo source), so a
    /// seed and a demo source for the same resource are distinct in the registry.
    #[must_use]
    pub fn id(&self) -> String {
        if self.demo {
            format!("{}/{}#demo", self.module, self.resource)
        } else {
            format!("{}/{}", self.module, self.resource)
        }
    }

    /// The parsed rows of this source (header-keyed maps).
    #[must_use]
    pub fn rows(&self) -> Vec<BTreeMap<String, String>> {
        parse_csv(self.csv)
    }
}

/// A seed source's registration in the link-time inventory.
///
/// [`register_seed!`](crate::register_seed) emits one `inventory::submit!` of this
/// per CSV file, so every source *linked into a binary* is discoverable through
/// [`registered_seeds`] — the same mechanism facades and admin resources use.
pub struct SeedRegistration {
    /// The registered source.
    pub source: SeedSource,
}

inventory::collect!(SeedRegistration);

/// Every seed/demo source registered via [`register_seed!`](crate::register_seed)
/// and linked into the current binary, sorted by `id()` for a deterministic order
/// regardless of link order.
///
/// A standalone tool (the `seiva` CLI) sees only what is compiled into it; the
/// composition-root binary that mounts the modules sees them all.
#[must_use]
pub fn registered_seeds() -> Vec<SeedSource> {
    let mut sources: Vec<SeedSource> = inventory::iter::<SeedRegistration>
        .into_iter()
        .map(|registration| registration.source)
        .collect();
    sources.sort_by_key(SeedSource::id);
    sources
}

/// The seed-source ids registered more than once — a conflict so a
/// double-registration fails loudly rather than loading the same file twice.
/// Returns the duplicated ids (each once), sorted; empty when consistent.
#[must_use]
pub fn seed_registry_conflicts() -> Vec<String> {
    let sources = registered_seeds();
    let mut conflicts = Vec::new();
    // `sources` is sorted by id, so duplicates are adjacent.
    let mut i = 0;
    while i < sources.len() {
        let id = sources[i].id();
        let mut j = i + 1;
        while j < sources.len() && sources[j].id() == id {
            j += 1;
        }
        if j - i > 1 {
            conflicts.push(id);
        }
        i = j;
    }
    conflicts
}

/// Registers a [`SeedSource`] in the link-time inventory. A module declares its
/// `seed/` file from a layer that reaches the data layer (`infrastructure`) and its
/// `demo/` file from the transport layer (`interface`); both expand to an
/// `inventory::submit!`, so the call must sit at item position (module scope).
#[macro_export]
macro_rules! register_seed {
    ($source:expr $(,)?) => {
        $crate::inventory::submit! {
            $crate::seed::SeedRegistration { source: $source }
        }
    };
}

/// A stable, deterministic 128-bit id for a seed row's textual key, so the same
/// `(scope, key)` always yields the same id across runs and platforms — the Odoo
/// "external id" without a mapping table.
///
/// `scope` is conventionally `"<module>.<resource>"` and `key` the row's `id`
/// column. The hash is FNV-1a over `scope`, a `0x1f` separator and `key`, so
/// `("a.b", "c")` and `("a", "b.c")` never collide. Pure integer arithmetic, so it
/// is reproducible everywhere (no `rand`, no platform hasher).
#[must_use]
pub fn external_id(scope: &str, key: &str) -> Id {
    // FNV-1a, 128-bit.
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let mut hash = OFFSET;
    for &byte in scope
        .as_bytes()
        .iter()
        .chain(std::iter::once(&0x1f))
        .chain(key.as_bytes())
    {
        hash ^= u128::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    Id(hash.to_be_bytes())
}

/// [`external_id`] as the `u128` every domain entity/DTO stores for its id.
#[must_use]
pub fn external_u128(scope: &str, key: &str) -> u128 {
    u128::from_be_bytes(external_id(scope, key).0)
}

/// Parses a CSV body into header-keyed rows. RFC 4180 subset: the first
/// non-comment, non-blank record is the header; fields may be quoted (with `""`
/// for a literal quote, and commas/newlines inside quotes); blank lines and lines
/// whose first field starts with `#` are skipped. A row with fewer fields than the
/// header simply omits the missing columns; extra fields are ignored.
#[must_use]
pub fn parse_csv(src: &str) -> Vec<BTreeMap<String, String>> {
    let records: Vec<Vec<String>> = parse_records(src)
        .into_iter()
        // Skip blank lines (a single empty field) and `#` comments.
        .filter(|record| !(record.len() == 1 && record[0].trim().is_empty()))
        .filter(|record| !record[0].trim_start().starts_with('#'))
        .collect();

    let mut iter = records.into_iter();
    let Some(header) = iter.next() else {
        return Vec::new();
    };
    let header: Vec<String> = header.into_iter().map(|h| h.trim().to_owned()).collect();

    iter.map(|row| header.iter().cloned().zip(row).collect())
        .collect()
}

/// Tokenises a CSV body into records of fields, honouring quoted fields. Internal
/// to [`parse_csv`]; separated so the state machine is unit-tested directly.
fn parse_records(src: &str) -> Vec<Vec<String>> {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = src.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' if field.is_empty() => in_quotes = true,
                ',' => record.push(std::mem::take(&mut field)),
                '\r' => {}
                '\n' => {
                    record.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut record));
                }
                _ => field.push(c),
            }
        }
    }
    // Flush a final field/record not terminated by a newline.
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn seed(module: &'static str, resource: &'static str, demo: bool) -> SeedSource {
        SeedSource {
            module,
            resource,
            csv: "id,title\n",
            demo,
        }
    }

    // One unique seed source and a deliberately duplicated one, into this test
    // binary's inventory.
    register_seed!(seed("catalog", "product", false));
    register_seed!(seed("orders", "order", true));
    register_seed!(seed("orders", "order", true));

    #[test]
    fn registered_sources_are_discoverable_and_sorted() {
        let ids: Vec<String> = registered_seeds().iter().map(SeedSource::id).collect();
        assert!(ids.contains(&"catalog/product".to_owned()));
        assert!(ids.contains(&"orders/order#demo".to_owned()));
        // Sorted by id: catalog precedes orders.
        let c = ids.iter().position(|i| i == "catalog/product").unwrap();
        let o = ids.iter().position(|i| i == "orders/order#demo").unwrap();
        assert!(c < o, "sources must be sorted: {ids:?}");
    }

    #[test]
    fn a_duplicate_registration_is_a_conflict() {
        let conflicts = seed_registry_conflicts();
        assert!(
            conflicts.contains(&"orders/order#demo".to_owned()),
            "the double-registered id must be reported: {conflicts:?}"
        );
        assert!(!conflicts.contains(&"catalog/product".to_owned()));
    }

    #[test]
    fn external_id_is_deterministic_and_scoped() {
        // Stable across calls.
        assert_eq!(
            external_id("catalog.product", "apple"),
            external_id("catalog.product", "apple")
        );
        // Different key or scope → different id.
        assert_ne!(
            external_id("catalog.product", "apple"),
            external_id("catalog.product", "pear")
        );
        assert_ne!(
            external_id("catalog.product", "apple"),
            external_id("orders.order", "apple")
        );
        // No scope/key boundary collision: ("a.b","c") != ("a","b.c").
        assert_ne!(external_id("a.b", "c"), external_id("a", "b.c"));
        // The u128 view matches the byte view.
        assert_eq!(
            external_u128("catalog.product", "apple"),
            u128::from_be_bytes(external_id("catalog.product", "apple").0)
        );
    }

    #[test]
    fn parse_csv_reads_headers_quotes_and_comments() {
        let src = "\
# a comment line, skipped
id,title,note

apple,Apple,\"red, sweet\"
pear,\"Pear \"\"Bartlett\"\"\",
";
        let rows = parse_csv(src);
        assert_eq!(rows.len(), 2, "blank + comment lines skipped: {rows:?}");
        assert_eq!(rows[0]["id"], "apple");
        assert_eq!(rows[0]["title"], "Apple");
        // Comma inside quotes stays one field.
        assert_eq!(rows[0]["note"], "red, sweet");
        // Escaped quotes.
        assert_eq!(rows[1]["title"], "Pear \"Bartlett\"");
        // A trailing empty field is present but empty.
        assert_eq!(rows[1]["note"], "");
    }

    #[test]
    fn parse_csv_empty_is_empty() {
        assert!(parse_csv("").is_empty());
        assert!(parse_csv("\n\n").is_empty());
        // Header only, no rows.
        assert!(parse_csv("id,title\n").is_empty());
    }
}
