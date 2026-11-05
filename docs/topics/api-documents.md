# API documents: OpenAPI, the docs UI, admin and proto

Seiva is API-first, so where Django has a template layer, Seiva **generates the
API's own documents** from the facade contracts — no hand-maintained spec.

## OpenAPI 3.1 (`/openapi.json`)

Every running service serves `GET /openapi.json`, generated from **every facade
linked into the binary** via the link-time inventory (no central list — OWASP
API9). It is *faithful*: a write body is the DTO directly (no `{cmd:…}` envelope),
and each DTO's real fields come from its `WireSchema` derive — not an opaque
placeholder (ADR 0007). Guarded operations carry `x-seiva-requires` and a bearer
security scheme.

Offline, the CLI prints the project's document:

```console
$ seiva openapi            # builds+runs the project, emits its OpenAPI; --compact for one line
```

Inside a project this documents *your* modules (via `SEIVA_EMIT=openapi` on the
composition root); outside a project it shows the built-in demo facade.

## The docs UI (`/docs`)

`GET /docs` serves an interactive API reference (Scalar) over the service's own
`/openapi.json` — the FastAPI `/docs` parity piece.

## The admin console (`/admin`) — an optional extension

The admin console is **not** part of the base framework (ADR 0008). A plain
Seiva backend answers `404` for `/admin` and carries none of its assets. The
console ships as a separate crate, `seiva-console-admin`, that a composition root
opts into explicitly:

```rust
// in your apps/<name>-api composition root, merge into `modules` BEFORE the
// framework layers wrap them (so /admin gets authn, headers and the rate limit):
let modules = seiva_console_admin::mount(modules);
let app = seiva_http::router_with_modules(health, &security, auth, modules);
```

`seiva ext add seiva-console-admin` inserts that line at the managed anchor for
you (ADR 0010).

Once mounted, `GET /admin` is a self-contained console (no CDN, strict
hash-based CSP) that renders each **registered, authorized** resource with its
operations and a try-form per write operation built from the DTO's schema, with
a bearer-token field.

A resource appears only when a module **registers** it with
`delonix_seiva::register_admin!` *and* the caller holds its `view` permission —
being in `/openapi.json` is never enough (ADR 0009). The console reads a
server-filtered `GET /admin/registry.json`, so an unauthorized caller sees
neither the resource nor its existence; write forms are hidden unless the caller
holds the create/change permission (and the server guard enforces it regardless).
A module that registers nothing stays private to the console.

```rust
// in the module's interface layer (e.g. examples/accounts/interface):
use delonix_seiva::admin::{AdminPermissions, AdminResource};
use delonix_seiva::register_admin;

register_admin!(AdminResource {
    namespace: "accounts", resource: "accounts", label: "Accounts",
    facade: "Accounts", version: "v1",
    list_fields: &["id", "username"], search_fields: &["username"],
    filters: &[], ordering: &["username"], page_size: 25,
    permissions: AdminPermissions {
        view: "accounts.read", create: "accounts.create", change: "", delete: "",
    },
});
```

It drives the contract's *operations*; for browsing/editing data rows use
`seiva db studio`. See [the admin](authentication.md) story for the auth model.

## proto3 (`seiva proto`)

```console
$ seiva proto              # the project's proto3 service document
```

Mirrors `seiva openapi`: inside a project it emits the project's facades as proto
services (`SEIVA_EMIT=proto`); outside, the built-in demo. gRPC transport serving
is a later phase; the proto document is generated from the same descriptor today.

## Why it never drifts

All four documents derive from the *same* facade descriptors and `WireSchema` the
server enforces at runtime. There is no second source of truth to keep in sync —
the documents are the contract, generated.

## Related

- [Facades](transport/facades.md) · [Validation](validation.md)
- [Wire schema](../reference/wire-schema.md) · [ADR 0007](../adr/0007-wire-schema-contract-fidelity.md)

---

*Part of the [Seiva documentation](../index.md).*
