//! Per-module admin registration — the descriptor a module declares so the
//! optional console (`seiva-console-admin`, ADR 0008/0009) may surface it.
//!
//! This lives in the framework crate, **not** in the console extension, on
//! purpose: a module's interface layer already depends on `seiva` (for
//! `#[controller]`/`#[facade]`), so it can register admin metadata without taking
//! a dependency on the admin UI crate. A module that never calls
//! [`register_admin!`](crate::register_admin) is simply absent from the registry
//! — **private to the console**. And when the console extension is not mounted,
//! the registrations are inert link-time data: no runtime cost, no behaviour.
//!
//! The registry is the **authorization gate for exposure**: the console shows a
//! resource only when it is registered here *and* the caller holds its `view`
//! permission. Presence in `/openapi.json` never authorises admin exposure
//! (OWASP API9 / API1): the linker including a facade does not put it in the
//! console.
//!
//! ```
//! use seiva::admin::{AdminPermissions, AdminResource};
//! use seiva::register_admin;
//!
//! register_admin!(AdminResource {
//!     namespace: "accounts",
//!     resource: "users",
//!     label: "Users",
//!     facade: "Accounts",
//!     version: "v1",
//!     list_fields: &["id", "username"],
//!     search_fields: &["username"],
//!     filters: &[],
//!     ordering: &["username"],
//!     page_size: 25,
//!     permissions: AdminPermissions {
//!         view: "accounts.read",
//!         create: "accounts.create",
//!         change: "",
//!         delete: "",
//!     },
//! });
//! ```

/// The permission a console caller must hold for each admin operation on a
/// resource. An **empty** string means the operation is not offered at all (the
/// console neither shows nor attempts it); a non-empty string is checked against
/// the caller's `Principal` (from `seiva-identity`) permissions. Authorization
/// is always enforced server-side by the module's `#[controller]` guard too; this
/// drives *exposure and UX*, it does not replace the guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdminPermissions {
    /// View (list + detail). An empty string hides the resource entirely.
    pub view: &'static str,
    /// Create a new record. Empty disables the create form.
    pub create: &'static str,
    /// Change an existing record. Empty disables edit.
    pub change: &'static str,
    /// Delete a record. Empty disables delete.
    pub delete: &'static str,
}

/// A module's admin resource: a stable `(namespace, resource)` identity, display
/// metadata and the per-operation permissions the console enforces.
///
/// The `(namespace, resource)` pair is the stable id and drives the predictable
/// route `/admin/{namespace}/{resource}`. Two registrations sharing it are a
/// conflict ([`admin_registry_conflicts`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdminResource {
    /// The owning module/namespace, e.g. `"inventory"`.
    pub namespace: &'static str,
    /// The resource id within the namespace, e.g. `"items"`. Matches the
    /// resource segment of the module's routes (`/{version}/{resource}`), so the
    /// console can pair the registration with the operations in `/openapi.json`.
    pub resource: &'static str,
    /// A human-readable label for the console (e.g. `"Catalogue items"`).
    pub label: &'static str,
    /// The backing facade's name (the `#[facade]` trait identifier).
    pub facade: &'static str,
    /// The backing facade's version, e.g. `"v1"`.
    pub version: &'static str,
    /// Columns to show in the list view, in order.
    pub list_fields: &'static [&'static str],
    /// Fields the console offers as a text search (server-side filtering is the
    /// module's responsibility; this is the allow-list the UI may expose).
    pub search_fields: &'static [&'static str],
    /// Fields the console offers as filters (an allow-list, never free-form).
    pub filters: &'static [&'static str],
    /// Fields the list view may be ordered by (an allow-list).
    pub ordering: &'static [&'static str],
    /// The list view's page size.
    pub page_size: u32,
    /// Per-operation permissions (empty string = operation not offered).
    pub permissions: AdminPermissions,
}

impl AdminResource {
    /// The console route for this resource: `/admin/{namespace}/{resource}`.
    #[must_use]
    pub fn path(&self) -> String {
        format!("/admin/{}/{}", self.namespace, self.resource)
    }

    /// The stable id `{namespace}/{resource}`.
    #[must_use]
    pub fn id(&self) -> String {
        format!("{}/{}", self.namespace, self.resource)
    }

    /// Validate an **untrusted** [`ListQuery`] against this resource's allow-lists,
    /// returning a [`ValidatedListQuery`] whose every field is already known-safe —
    /// or a [`ListQueryError`] naming the first offending field.
    ///
    /// This is what finally makes the registry's `search_fields`/`filters`/`ordering`
    /// metadata **enforced** rather than advisory (their doc-comments say server-side
    /// filtering was "the module's responsibility"): a client cannot search a resource
    /// that offers no search, filter on a field outside [`filters`](Self::filters), or
    /// order by a column outside [`ordering`](Self::ordering) — the exact lever an
    /// injection would reach for. It is **fail-closed**: an unknown field is a hard
    /// error, never silently dropped. `page_size` is clamped to the registered
    /// [`page_size`](Self::page_size) maximum and the offset is derived, so a client
    /// cannot ask for an unbounded page.
    ///
    /// It validates the *shape* of the query, not the *values*: a filter value is
    /// passed through verbatim for the caller to bind as a parameter (never to
    /// interpolate), exactly as the typed DORM QuerySet does.
    pub fn validate_query(&self, query: &ListQuery) -> Result<ValidatedListQuery, ListQueryError> {
        // `page_size` is the registered maximum; a request may ask for less but never
        // more, and 0/absent falls back to it. `max(1)` guards a resource registered
        // with `page_size = 0`.
        let max_page_size = self.page_size.max(1);
        let page_size = match query.page_size {
            Some(requested) if requested > 0 => requested.min(max_page_size),
            _ => max_page_size,
        };
        // Pages are 1-based; page 0 is treated as page 1. The offset saturates so a
        // huge page number can never overflow.
        let page = query.page.max(1);
        let offset = page.saturating_sub(1).saturating_mul(page_size);

        // Search is offered only if the resource registered any searchable field.
        let search = match &query.search {
            Some(text) if !text.is_empty() => {
                if self.search_fields.is_empty() {
                    return Err(ListQueryError::SearchNotAllowed);
                }
                Some(text.clone())
            }
            _ => None,
        };

        // Ordering may name only a registered field (fail-closed): an arbitrary
        // `order_by` is exactly the ORDER BY-column injection this guards against.
        let order_by = match &query.order_by {
            Some(field) if !field.is_empty() => {
                if !self.ordering.contains(&field.as_str()) {
                    return Err(ListQueryError::UnknownOrderField(field.clone()));
                }
                Some((field.clone(), query.order_dir))
            }
            _ => None,
        };

        // Every filter field must be in the allow-list; the value rides through for
        // the caller to bind (not interpolate).
        let mut filters = Vec::with_capacity(query.filters.len());
        for filter in &query.filters {
            if !self.filters.contains(&filter.field.as_str()) {
                return Err(ListQueryError::UnknownFilterField(filter.field.clone()));
            }
            filters.push((filter.field.clone(), filter.value.clone()));
        }

        Ok(ValidatedListQuery {
            page,
            page_size,
            offset,
            search,
            order_by,
            filters,
        })
    }
}

/// The sort direction of an admin list query. Defaults to [`Asc`](SortDirection::Asc).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortDirection {
    /// Ascending (the default).
    #[default]
    Asc,
    /// Descending.
    Desc,
}

/// One `field = value` filter clause of a [`ListQuery`]. The `field` is untrusted
/// until [`AdminResource::validate_query`] checks it against the resource's
/// [`filters`](AdminResource::filters) allow-list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterClause {
    /// The field to filter on — validated against the resource's allow-list.
    pub field: String,
    /// The value to match; carried through verbatim for the caller to **bind** as a
    /// query parameter, never to interpolate into SQL.
    pub value: String,
}

/// A raw, **untrusted** admin list query as it arrives from a client (the console or
/// an API caller): pagination, a free-text search, an ordering field + direction, and
/// `field = value` filters. Every field name here is attacker-controlled until
/// [`AdminResource::validate_query`] checks it against the resource's allow-lists.
#[derive(Debug, Clone, Default)]
pub struct ListQuery {
    /// The 1-based page number; `0` is treated as page `1`.
    pub page: u32,
    /// The requested page size; clamped to the resource's registered maximum, and
    /// falling back to it when absent or `0`.
    pub page_size: Option<u32>,
    /// A free-text search term; honoured only if the resource registered any
    /// [`search_fields`](AdminResource::search_fields).
    pub search: Option<String>,
    /// The field to order by; must be in the resource's
    /// [`ordering`](AdminResource::ordering) allow-list.
    pub order_by: Option<String>,
    /// The sort direction for [`order_by`](Self::order_by).
    pub order_dir: SortDirection,
    /// The `field = value` filters; each field must be in the resource's
    /// [`filters`](AdminResource::filters) allow-list.
    pub filters: Vec<FilterClause>,
}

impl ListQuery {
    /// Build a [`ListQuery`] from **already-decoded** query-string pairs — what an
    /// HTTP extractor hands over after URL-decoding `?page=2&filter.status=active&…`.
    ///
    /// The convention:
    /// - `page` / `page_size` — unsigned integers; a missing or unparsable value
    ///   leaves the default (`page` 0, `page_size` `None`);
    /// - `search` — a free-text term (an empty value is dropped);
    /// - `order_by` — the field to order by (empty dropped); `order_dir` — `desc`
    ///   (any case) for descending, anything else ascending;
    /// - `filter.<field>=<value>` — one [`FilterClause`] per such pair.
    ///
    /// Unknown keys are ignored and scalar keys are last-wins. **Nothing is trusted
    /// here**: this only shapes the raw pairs into the typed query —
    /// [`AdminResource::validate_query`] is what then checks every field name against
    /// the resource's allow-lists, fail-closed.
    #[must_use]
    pub fn from_pairs(pairs: &[(String, String)]) -> Self {
        let mut query = ListQuery::default();
        for (key, value) in pairs {
            match key.as_str() {
                "page" => {
                    if let Ok(page) = value.parse() {
                        query.page = page;
                    }
                }
                "page_size" => query.page_size = value.parse().ok(),
                "search" => query.search = (!value.is_empty()).then(|| value.clone()),
                "order_by" => query.order_by = (!value.is_empty()).then(|| value.clone()),
                "order_dir" => {
                    query.order_dir = if value.eq_ignore_ascii_case("desc") {
                        SortDirection::Desc
                    } else {
                        SortDirection::Asc
                    };
                }
                other => {
                    if let Some(field) = other.strip_prefix("filter.") {
                        if !field.is_empty() {
                            query.filters.push(FilterClause {
                                field: field.to_owned(),
                                value: value.clone(),
                            });
                        }
                    }
                }
            }
        }
        query
    }
}

/// A [`ListQuery`] that has passed [`AdminResource::validate_query`]: every field is
/// already checked against the resource's allow-lists, so a caller can build the
/// actual data query (e.g. the typed DORM QuerySet) from it without re-checking —
/// the fields are known-safe and the page is bounded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedListQuery {
    /// The 1-based page number (`>= 1`).
    pub page: u32,
    /// The effective page size (`1 ..= resource.page_size`).
    pub page_size: u32,
    /// The row offset, `(page - 1) * page_size`, saturating.
    pub offset: u32,
    /// The search term, present only when the resource allows searching.
    pub search: Option<String>,
    /// The ordering, with a field guaranteed to be in the allow-list.
    pub order_by: Option<(String, SortDirection)>,
    /// The filters, each field guaranteed to be in the allow-list; values are to be
    /// **bound**, never interpolated.
    pub filters: Vec<(String, String)>,
}

/// Why an admin [`ListQuery`] was rejected by [`AdminResource::validate_query`] —
/// always because a client named a field outside the resource's allow-lists, which
/// is refused fail-closed rather than silently dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListQueryError {
    /// A filter named a field not in the resource's [`filters`](AdminResource::filters).
    UnknownFilterField(String),
    /// An `order_by` named a field not in the resource's
    /// [`ordering`](AdminResource::ordering).
    UnknownOrderField(String),
    /// A search was requested on a resource that registered no
    /// [`search_fields`](AdminResource::search_fields).
    SearchNotAllowed,
}

impl std::fmt::Display for ListQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListQueryError::UnknownFilterField(field) => {
                write!(f, "cannot filter by `{field}`: not an allowed filter field")
            }
            ListQueryError::UnknownOrderField(field) => {
                write!(
                    f,
                    "cannot order by `{field}`: not an allowed ordering field"
                )
            }
            ListQueryError::SearchNotAllowed => {
                write!(f, "this resource does not offer text search")
            }
        }
    }
}

impl std::error::Error for ListQueryError {}

/// A resource's registration in the link-time inventory.
///
/// [`register_admin!`](crate::register_admin) emits one `inventory::submit!` of
/// this per resource, so every resource *linked into a binary* is discoverable
/// through [`registered_admin_resources`] without a hand-maintained list — the
/// same discovery mechanism facades use ([`FacadeRegistration`](crate::contract::FacadeRegistration)).
pub struct AdminRegistration {
    /// The registered resource.
    pub resource: AdminResource,
}

inventory::collect!(AdminRegistration);

/// Every admin resource registered via [`register_admin!`](crate::register_admin)
/// and linked into the current binary, sorted by `(namespace, resource)` so the
/// order is deterministic regardless of link order.
///
/// A standalone tool (the `seiva` CLI) sees only what is compiled into it; the
/// runtime binary that mounts the modules sees them all, so the console it hosts
/// lists the complete set.
#[must_use]
pub fn registered_admin_resources() -> Vec<AdminResource> {
    let mut resources: Vec<AdminResource> = inventory::iter::<AdminRegistration>
        .into_iter()
        .map(|registration| registration.resource)
        .collect();
    resources.sort_by(|a, b| (a.namespace, a.resource).cmp(&(b.namespace, b.resource)));
    resources
}

/// The `(namespace, resource)` ids registered more than once — a conflict the
/// console surfaces so a double-registration (or two modules claiming the same
/// route) fails loudly at startup rather than silently shadowing one another.
///
/// Returns the duplicated ids (each once), sorted; empty when the registry is
/// consistent.
#[must_use]
pub fn admin_registry_conflicts() -> Vec<String> {
    let resources = registered_admin_resources();
    let mut conflicts = Vec::new();
    // `resources` is sorted by (namespace, resource), so duplicates are adjacent.
    let mut i = 0;
    while i < resources.len() {
        let mut j = i + 1;
        while j < resources.len()
            && resources[j].namespace == resources[i].namespace
            && resources[j].resource == resources[i].resource
        {
            j += 1;
        }
        if j - i > 1 {
            conflicts.push(resources[i].id());
        }
        i = j;
    }
    conflicts
}

/// Registers an [`AdminResource`] in the link-time inventory. A module's
/// interface layer calls this (typically from an `admin.rs`) to make the resource
/// visible to the console; a module that does not is private to the console.
///
/// It expands to an `inventory::submit!`, so it must appear at item position
/// (module scope), not inside a function.
#[macro_export]
macro_rules! register_admin {
    ($resource:expr $(,)?) => {
        $crate::inventory::submit! {
            $crate::admin::AdminRegistration { resource: $resource }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn res(namespace: &'static str, resource: &'static str) -> AdminResource {
        AdminResource {
            namespace,
            resource,
            label: "L",
            facade: "F",
            version: "v1",
            list_fields: &["id"],
            search_fields: &[],
            filters: &[],
            ordering: &[],
            page_size: 25,
            permissions: AdminPermissions {
                view: "f.read",
                create: "f.create",
                change: "",
                delete: "",
            },
        }
    }

    // One unique resource and a deliberately duplicated pair, registered into
    // this test binary's inventory.
    register_admin!(res("catalogue", "items"));
    register_admin!(res("billing", "invoices"));
    register_admin!(res("billing", "invoices"));

    #[test]
    fn registered_resources_are_discoverable_and_sorted() {
        let ids: Vec<String> = registered_admin_resources()
            .iter()
            .map(AdminResource::id)
            .collect();
        assert!(ids.contains(&"catalogue/items".to_owned()));
        assert!(ids.contains(&"billing/invoices".to_owned()));
        // Sorted by (namespace, resource): billing precedes catalogue.
        let b = ids.iter().position(|i| i == "billing/invoices").unwrap();
        let c = ids.iter().position(|i| i == "catalogue/items").unwrap();
        assert!(b < c, "resources must be sorted: {ids:?}");
    }

    #[test]
    fn a_duplicate_registration_is_a_conflict() {
        let conflicts = admin_registry_conflicts();
        assert!(
            conflicts.contains(&"billing/invoices".to_owned()),
            "the double-registered id must be reported: {conflicts:?}"
        );
        // The unique one is not a conflict.
        assert!(!conflicts.contains(&"catalogue/items".to_owned()));
    }

    #[test]
    fn path_and_id_are_predictable() {
        let r = res("inventory", "items");
        assert_eq!(r.path(), "/admin/inventory/items");
        assert_eq!(r.id(), "inventory/items");
    }

    // --- A2 slice 1: ListQuery allow-list validation -------------------------

    /// A resource with populated allow-lists, to exercise `validate_query`.
    const fn res_with_lists() -> AdminResource {
        AdminResource {
            namespace: "catalogue",
            resource: "items",
            label: "Items",
            facade: "Catalogue",
            version: "v1",
            list_fields: &["id", "name", "status"],
            search_fields: &["name"],
            filters: &["status", "kind"],
            ordering: &["name", "created_at"],
            page_size: 25,
            permissions: AdminPermissions {
                view: "catalogue.read",
                create: "catalogue.create",
                change: "catalogue.change",
                delete: "",
            },
        }
    }

    fn filter(field: &str, value: &str) -> FilterClause {
        FilterClause {
            field: field.to_owned(),
            value: value.to_owned(),
        }
    }

    #[test]
    fn a_fully_allow_listed_query_validates_and_bounds_the_page() {
        let r = res_with_lists();
        let query = ListQuery {
            page: 3,
            page_size: Some(10),
            search: Some("widget".to_owned()),
            order_by: Some("name".to_owned()),
            order_dir: SortDirection::Desc,
            filters: vec![filter("status", "active"), filter("kind", "tool")],
        };

        let valid = r
            .validate_query(&query)
            .expect("every field is allow-listed");
        assert_eq!(valid.page, 3);
        assert_eq!(valid.page_size, 10);
        assert_eq!(valid.offset, 20, "(page 3 - 1) * 10");
        assert_eq!(valid.search.as_deref(), Some("widget"));
        assert_eq!(
            valid.order_by,
            Some(("name".to_owned(), SortDirection::Desc))
        );
        assert_eq!(
            valid.filters,
            vec![
                ("status".to_owned(), "active".to_owned()),
                ("kind".to_owned(), "tool".to_owned()),
            ],
            "values ride through verbatim for the caller to bind"
        );
    }

    #[test]
    fn page_size_is_clamped_to_the_registered_maximum_and_page_zero_is_one() {
        let r = res_with_lists(); // page_size max = 25
        let query = ListQuery {
            page: 0,
            page_size: Some(1000),
            ..ListQuery::default()
        };
        let valid = r.validate_query(&query).unwrap();
        assert_eq!(
            valid.page_size, 25,
            "a client cannot ask for more than the max"
        );
        assert_eq!(valid.page, 1, "page 0 is treated as page 1");
        assert_eq!(valid.offset, 0);

        // Absent page_size falls back to the registered maximum.
        let valid = r.validate_query(&ListQuery::default()).unwrap();
        assert_eq!(valid.page_size, 25);
    }

    #[test]
    fn a_filter_on_an_unregistered_field_is_refused_fail_closed() {
        let r = res_with_lists();
        let query = ListQuery {
            filters: vec![filter("password_hash", "x")],
            ..ListQuery::default()
        };
        assert_eq!(
            r.validate_query(&query),
            Err(ListQueryError::UnknownFilterField(
                "password_hash".to_owned()
            )),
            "an off-list filter is a hard error, never silently dropped"
        );
    }

    #[test]
    fn an_order_by_outside_the_allow_list_is_refused() {
        let r = res_with_lists();
        let query = ListQuery {
            order_by: Some("secret_column".to_owned()),
            ..ListQuery::default()
        };
        assert_eq!(
            r.validate_query(&query),
            Err(ListQueryError::UnknownOrderField(
                "secret_column".to_owned()
            )),
            "an arbitrary ORDER BY column is the injection this guards against"
        );
    }

    #[test]
    fn search_on_a_resource_that_offers_none_is_refused() {
        // `res(..)` registers empty search_fields.
        let r = res("billing", "invoices");
        let query = ListQuery {
            search: Some("anything".to_owned()),
            ..ListQuery::default()
        };
        assert_eq!(
            r.validate_query(&query),
            Err(ListQueryError::SearchNotAllowed)
        );
        // An empty/absent search on the same resource is fine (no search requested).
        assert!(r.validate_query(&ListQuery::default()).is_ok());
    }

    // --- A2 slice 4: parsing query-string pairs into a ListQuery -------------

    fn pair(k: &str, v: &str) -> (String, String) {
        (k.to_owned(), v.to_owned())
    }

    #[test]
    fn from_pairs_maps_the_query_string_convention() {
        let pairs = vec![
            pair("page", "3"),
            pair("page_size", "10"),
            pair("search", "widget"),
            pair("order_by", "name"),
            pair("order_dir", "DESC"), // case-insensitive
            pair("filter.status", "active"),
            pair("filter.kind", "tool"),
            pair("unknown", "ignored"), // unknown key dropped
            pair("filter.", "dropped"), // empty filter field dropped
        ];
        let query = ListQuery::from_pairs(&pairs);

        assert_eq!(query.page, 3);
        assert_eq!(query.page_size, Some(10));
        assert_eq!(query.search.as_deref(), Some("widget"));
        assert_eq!(query.order_by.as_deref(), Some("name"));
        assert_eq!(query.order_dir, SortDirection::Desc);
        assert_eq!(
            query.filters,
            vec![filter("status", "active"), filter("kind", "tool")],
            "only well-formed `filter.<field>` pairs become clauses"
        );
    }

    #[test]
    fn from_pairs_defaults_are_sane() {
        // Empty input: all defaults.
        let query = ListQuery::from_pairs(&[]);
        assert_eq!(query.page, 0);
        assert_eq!(query.page_size, None);
        assert!(query.search.is_none());
        assert_eq!(query.order_dir, SortDirection::Asc, "default is ascending");

        // Unparsable numbers and an empty search leave the defaults.
        let query = ListQuery::from_pairs(&[
            pair("page", "notnum"),
            pair("page_size", "-1"),
            pair("search", ""),
        ]);
        assert_eq!(query.page, 0);
        assert_eq!(query.page_size, None);
        assert!(query.search.is_none());
    }

    #[test]
    fn parsed_query_then_validates_against_the_resource_allow_lists() {
        let r = res_with_lists();

        // A fully allow-listed query string round-trips to a ValidatedListQuery.
        let ok = ListQuery::from_pairs(&[
            pair("page", "2"),
            pair("page_size", "5"),
            pair("order_by", "name"),
            pair("filter.status", "active"),
        ]);
        let valid = r.validate_query(&ok).expect("all fields are allow-listed");
        assert_eq!(valid.page, 2);
        assert_eq!(valid.page_size, 5);
        assert_eq!(valid.offset, 5);
        assert_eq!(
            valid.order_by,
            Some(("name".to_owned(), SortDirection::Asc))
        );
        assert_eq!(
            valid.filters,
            vec![("status".to_owned(), "active".to_owned())]
        );

        // An off-list filter from the query string is refused fail-closed.
        let bad = ListQuery::from_pairs(&[pair("filter.secret", "x")]);
        assert_eq!(
            r.validate_query(&bad),
            Err(ListQueryError::UnknownFilterField("secret".to_owned()))
        );
    }
}
