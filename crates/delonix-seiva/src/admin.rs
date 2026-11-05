//! Per-module admin registration — the descriptor a module declares so the
//! optional console (`seiva-console-admin`, ADR 0008/0009) may surface it.
//!
//! This lives in the framework crate, **not** in the console extension, on
//! purpose: a module's interface layer already depends on `delonix-seiva` (for
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
//! use delonix_seiva::admin::{AdminPermissions, AdminResource};
//! use delonix_seiva::register_admin;
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
}

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
}
