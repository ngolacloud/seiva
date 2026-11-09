# Seiva Console Admin — Architecture Review (the integration target)

**Subject:** Delonix Seiva, `/home/walter/workspaces/NgolaCloud/projects/seiva`, commit `d1effe9`. A Rust (axum/tower) workspace of 22 crates. The integration target is the **Console Admin** (`crates/seiva-console-admin`).

---

## 1. What the Console Admin is today

The Console Admin is a **single, self-contained, server-rendered page** emitted by one Rust crate (`crates/seiva-console-admin/src/lib.rs`, 699 LOC). It is **not** a client-side JS application and there is **no incumbent JS framework or bundler to displace** — the repository's only `package.json`/`node_modules` belong to the Playwright E2E harness (`e2e/admin/`) and a VS Code extension (`editors/vscode/`); there is no `vite`/`webpack`/`rollup`/`tsconfig` anywhere.

The page is built from three inline Rust string constants served verbatim:
- `ADMIN_CSS` (`lib.rs:83-117`) — stylesheet.
- `ADMIN_JS` (`lib.rs:124-401`) — ~270 lines of hand-written vanilla JS.
- `ADMIN_SHELL` (`lib.rs:406-426`) — the HTML skeleton with `{css}`/`{js}` substituted once (`admin_html`, `:430-438`).

It is an **OpenAPI-driven operations + data console**: on load it fetches exactly two same-origin endpoints — `/admin/registry.json` (authz-filtered resource list with `can` flags + list/search/ordering/page-size metadata) and `/openapi.json` (the contract and WireSchema shapes) — groups operations by the `/{version}/{resource}` path shape, classifies them into CRUD roles, and renders tables/forms. A resource with a `list` operation gets a full admin table (list + create + per-row edit/delete with optimistic-concurrency `version`); others get operation forms.

## 2. The security model it already enforces (the bar StillJS must clear)

This is the crux of the whole audit. The Console Admin is engineered to a standard StillJS cannot meet:

1. **Strict, hash-pinned CSP** (`admin_csp`, `lib.rs:452-469`): `default-src 'none'; script-src 'sha256-…'; style-src 'sha256-…'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. **No `unsafe-inline`, no `unsafe-eval`.** The inline `<script>`/`<style>` are pinned by SHA-256 computed from the exact bytes served, so policy and payload can never drift.
2. **Output encoding everywhere:** `esc()` (`lib.rs:125`) HTML-escapes every interpolated value before it reaches `innerHTML`; dynamic nodes are built with `createElement`/`setAttribute`.
3. **No inline event handlers:** every handler is attached via `addEventListener` (`lib.rs:195,261,267-268,282,294,298,320-321,399`).
4. **App-wide secure baseline:** `seiva-http`'s `security_headers` tower middleware sets `default-src 'none'; frame-ancestors 'none'` plus `nosniff`, `X-Frame-Options: DENY`, HSTS, `Referrer-Policy: no-referrer` on **every** response (`crates/seiva-http/src/security.rs:99-131`), asserted even on `429`s. CORS is same-origin-only by default (`security.rs:133-169`). The one relaxed route is `/docs` (Scalar from a CDN, `docs.rs:46-48`) — a deliberate, isolated exception.
5. **Backend-enforced authorization:** `Principal` (`crates/seiva-identity/src/lib.rs:37-47`) is built only by an `Authenticator` from verified credentials. The `#[seiva::controller]` macro wraps each route in `permission_guard` keyed on the method's `#[requires("…")]` (`crates/seiva-macros/src/lib.rs:400-465`), which reads the `Principal` from request extensions (missing → anonymous → deny) and returns RFC-9457 `403` (`crates/seiva-http/src/guard.rs:132-156`). Object-level access uses fail-closed `Scoped<T>`/`require_owns` (`guard.rs:37-121`). The console does **no** client authz: its `can` flags are explicitly presentational (`console-admin/src/lib.rs:507-528`), and `/admin/registry.json` is filtered server-side by the caller's `Principal` (fail-closed: no token → nothing).

## 3. Repository shape (context)

- **Crates (22):** `delonix-seiva` (public framework entrypoint, re-exports `admin`/`contract`/`schema`), `seiva-http` (transport, security headers, guard), `seiva-console-admin` (the console), `seiva-identity` (Principal/Authenticator, OIDC/JWT via JWKS), `seiva-openapi` (OpenAPI 3.1 from facade descriptors + WireSchema), `seiva-auth` (Argon2id), `seiva-macros` (`#[seiva::facade]`/`#[seiva::controller]`), `seiva-server`, `seiva-cli`, plus messaging/ratelimit/webhooks/cqrs/grpc/config/ports/testing and the DORM-coupled (workspace-excluded) data crates.
- **`apps/`:** composition-root proof binaries — `admin-e2e` (console + `accounts` module behind a `TokenAuthenticator` on `127.0.0.1:8099`), `admin-data-e2e` (DORM-backed `Catalog` CRUD, excluded), `dorm-e2e`.
- **`e2e/admin/`:** Playwright-over-system-Chrome browser tests (`admin.mjs`, `catalog.mjs`); `run.sh` builds `admin-e2e`, serves it, drives the real page, asserts fail-closed behaviour and that passwords are not echoed.
- **`benchmarks/`:** Rust-vs-Python micro-benchmark harness. **`docs/`:** MkDocs site.

## 4. Testing stack an integration would reuse

1. **Rust `#[test]`/`#[tokio::test]`** exercising the axum router via `tower::ServiceExt::oneshot` (e.g. `security.rs:171-305`, `console-admin/src/lib.rs` registry tests `:644-698`).
2. **Playwright over system Chrome** (`channel: "chrome"`, `PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1`), driving `GET /admin` end-to-end.

No new test framework is needed for any integration work.

## 5. What Seiva already does correctly (do not duplicate)

- Contract-driven UI generation from `/openapi.json` + WireSchema — the admin is auto-generated, Django-admin-style, with **zero per-module frontend code**.
- A complete, verified secure-rendering implementation (escape + strict CSP + addEventListener) — the exact properties StillJS lacks.
- Server-side authz, fail-closed registry filtering, optimistic concurrency, pagination/search/ordering driven by server-declared allow-lists (ADR 0009).

## 6. Implication for the StillJS question

The target is not a greenfield SPA needing a framework; it is a **deliberately dependency-free, CSP-hardened, server-rendered console**. Any client engine introduced here must (a) run under `admin_csp()` without `unsafe-eval`/`unsafe-inline`, (b) preserve output encoding, and (c) consume the registry/OpenAPI contract as read-only presentational input. StillJS fails (a) and (b) by construction. See `SEIVA_COMPATIBILITY_MATRIX.md` and `ARCHITECTURAL_DECISION_RECORD.md`.
