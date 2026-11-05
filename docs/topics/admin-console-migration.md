# Migrating to the admin console extension

As of ADR 0008 the admin console is no longer part of the Seiva base. If you
relied on `GET /admin` being served automatically, this page shows what changed
and how to restore it.

## What changed

- **Before:** every Seiva backend served `/admin` (the console lived in
  `seiva-http`).
- **After:** the base framework carries no admin code. `GET /admin` is `404`.
  The console ships as the optional crate **`seiva-console-admin`**, mounted
  explicitly by your composition root.
- The console's `Content-Security-Policy` is now strict: the inline `<style>`
  and `<script>` are pinned by SHA-256 hash; `unsafe-inline` is gone.

## Restoring `/admin` (two steps + a recompile)

A declared dependency alone does not mount routes — you link the crate **and**
compose it, then recompile.

1. **Add the dependency** to your composition-root package (the `apps/<name>-api`
   crate, *not* a `domain`/`application` layer):

   ```toml
   [dependencies]
   seiva-console-admin = "0.1"
   ```

   With the independent DX tool this is `dx add seiva-console-admin`; with Cargo
   it is the line above. Either way the integration is explicit and auditable.

2. **Mount it** where you build the application router, after merging your
   modules:

   ```rust
   // before: let app = seiva_http::router_with_modules(health, &security, auth, modules);
   let app = seiva_http::router_with_modules(health, &security, auth, modules);
   let app = seiva_console_admin::mount(app); // GET /admin now serves the console
   ```

3. **Recompile.** The route appears only after a rebuild — by design.

That is all. `seiva_console_admin::mount` adds exactly `GET /admin`; mounting it
twice is a build-time error (axum rejects the duplicate route), so a double-mount
surfaces at startup, never silently.

## If you want a backend with no admin

Do nothing: don't add the crate. `/admin` is `404`, and `cargo tree` shows no
`seiva-console-admin` in the backend's dependency graph.

## Compatibility

| | Base framework | `seiva-console-admin` |
| --- | --- | --- |
| `0.1.x` | `/admin` → 404 | mount with `seiva_console_admin::mount(router)` |

The console reads your service's live `/openapi.json`, so it tracks whatever
modules the backend mounts — no per-console configuration. (Per-module
`--admin` registration, model-row CRUD and server-side console authorization are
planned increments; see ADR 0008 and the roadmap's P1-5.)

## Related

- [ADR 0008](../adr/0008-admin-console-as-an-extension.md)
- [API documents](api-documents.md)
