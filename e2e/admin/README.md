# Admin console browser E2E (§10)

A real-browser end-to-end for the admin console: it drives a **headless system
Chrome** against a running Seiva service (the `admin-e2e` composition root — the
`accounts` module + `seiva-console-admin` + a token authenticator) and asserts
the core admin flow:

1. the console page loads;
2. **no token → no resources** (the registry is server-filtered and fail-closed);
3. a valid bearer reveals the registered, authorized `accounts` resource;
4. the create form submits to a `2xx`, echoing the username and **not** leaking
   the password.

It also guards the fix that the registry fetch must carry the bearer (otherwise
the server sees an anonymous caller and returns nothing).

## Run

Needs **Node** and **Google Chrome** on the host. Playwright uses the system
Chrome (`channel: "chrome"`) — no browser is downloaded.

```console
$ ./run.sh
```

`run.sh` builds and starts `admin-e2e`, installs Playwright (browser download
skipped), runs `admin.mjs`, and stops the server. Override the address with
`ADMIN_E2E_ADDR=127.0.0.1:PORT`.

## CI

This is **on-demand**, not part of `cargo test` or the default CI (which has no
browser). The `admin-e2e` server crate *is* built by CI (so it cannot rot); the
browser step runs where a Chrome is available.

## Row CRUD (`catalog-run.sh`)

`catalog.mjs` (run by `catalog-run.sh`) proves the console's **row CRUD** against
a real DORM-backed module (`apps/admin-data-e2e`'s `Catalog`) on PostgreSQL:
the resource renders as a table, then **create → it appears in the list → edit →
delete → it is gone**, all through the module's contract operations. Needs Node,
Chrome and a reachable PostgreSQL (DSN via `ADMIN_DATA_E2E_DSN`).

```console
$ ADMIN_DATA_E2E_DSN='host=/var/run/postgresql dbname=seiva_catalog_e2e' ./catalog-run.sh
```

## Not yet covered

The §10 list also names filter, version/ETag conflict, cross-tenant denial and
extension removal. These are follow-ups as the console grows pagination/search,
optimistic-concurrency and tenant-scoped reads (ADR 0009).
