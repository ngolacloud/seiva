# Part 10: An admin console, with no frontend written

A back office needs an admin UI, and you should not have to build one by hand.
Seiva's **admin console** (`seiva-console-admin`) is an **extension** of the base
(ADR 0008): mount it and every module that registers an admin resource appears,
auto-CRUD, guarded by the same permissions as its API. You write no frontend.

The console is an extension, never a dependency: a service built without it has no
`/admin` route and no console code in the binary. You opt in.

## Register a resource on a module

A module exposes itself to the console by registering an admin resource — this is
what `seiva module new --admin` scaffolds, and what each shop module already does
in its interface layer:

```rust
// modules/catalog/interface/src/lib.rs
register_admin! {
    namespace: "catalog",
    resource:  "catalogs",
    label:     "Products",
    view:      "catalog.read",     // who may list/see it
    create:    "catalog.manage",   // who may add
}
```

The console reads these registrations at link time and shows a caller **only** the
resources they hold the `view` permission for — the same deny-by-default rule as
the API (Part 5). The `can.*` flags only hide affordances; the backend still
enforces every operation, so the console can never grant access the contract
denies.

## Mount the console in the composition root

In `apps/shop-api` the console is merged into the modules router *before* the
framework layers, so the authenticator populates the `Principal` the console reads:

```rust
// apps/shop-api/src/main.rs
let modules = catalog_interface::catalog_router(/* … */)
    .merge(orders_interface::orders_router(/* … */))
    // … the other three modules …
    ;

let modules = seiva_console_admin::mount(modules);     // ← the extension

let app = router_with_modules(health, &HttpSecurity::default(), Some(auth), modules);
```

## See it

```console
$ cargo run -p shop-api          # http://127.0.0.1:8090
# open http://127.0.0.1:8090/admin and enter a bearer token
```

With a token holding `catalog.read`/`catalog.manage` (and the other modules'
permissions), the console lists Products, Orders, Inventory, Payments and
Notifications, each with a data table and create/edit forms — generated from the
registrations and the OpenAPI document, served under a strict Content-Security-
Policy (no inline scripts, every value escaped).

## Where the code lives

```text
modules/<module>/interface/src/lib.rs   # register_admin! { … }
apps/shop-api/src/main.rs               # seiva_console_admin::mount(modules)
```

## Build and test

```console
$ cargo run -p shop-api &
$ curl -s -o /dev/null -w '%{http_code}\n' -H 'authorization: Bearer <token>' localhost:8090/admin   # 200
```

The framework ships a browser E2E (`e2e/admin`) that authenticates, lists the
authorized resource and drives a create form end to end on headless Chrome — the
pattern to copy for your own admin tests.

## Live-reload while you work

```console
$ seiva start --watch
```

## Next

[Part 11: A separate dxjs frontend, over CORS](part-11-a-separate-dxjs-frontend.md)
— when you want a bespoke storefront instead of (or alongside) the admin console.

---

*Part of the [Seiva documentation](../../index.md).*
