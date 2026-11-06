# shop-storefront — a separate dxjs frontend for the shop API

A small [dxjs](https://github.com/ngolacloud/dxjs) single-page storefront for the
Seiva `shop-api` (tutorial
[Part 11](../../docs/getting-started/shop/part-11-a-separate-dxjs-frontend.md)). It
runs on **its own origin** and talks to the API **cross-origin over CORS** — the
decoupled-frontend pattern.

## Prerequisites

- **Node ≥ 22** (dxjs runs TypeScript directly; the bundle is built with esbuild).
- The **dxjs** repo checked out alongside this one (`../../../dxjs`), because dxjs
  is a private, unpublished package consumed as a sibling — the same way DORM is.
  Override its location with `DXJS_DIR`.

## Run it

```sh
# 1. The API, allowing this storefront's origin (CORS):
SHOP_API_CORS_ORIGINS=http://127.0.0.1:5173 cargo run -p shop-api   # :8090

# 2. Build + serve the storefront on its own origin:
cd examples/shop-storefront
node build.mjs            # → dist/app.js  (esbuild bundle of the dxjs app)
node serve.mjs            # → http://127.0.0.1:5173
```

Open <http://127.0.0.1:5173>: the catalogue loads from the API, you can add a
product and place an order. The bearer token field authorizes the calls (the demo
API accepts `shop-token`). All business logic stays on the server.

## Files

- `src/main.ts` — the dxjs app (`@dxjs/core` signals, `@dxjs/dom` `html`,
  `@dxjs/component` `mount`, `@dxjs/transport` `createClient`/`resource`).
- `build.mjs` — esbuild bundle, resolving `@dxjs/*` from the sibling dxjs repo.
- `serve.mjs` — a static server on its own origin.
- `index.html` — the shell that loads `dist/app.js`.
