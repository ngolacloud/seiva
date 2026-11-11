# EVIDENCE — Dependency & Supply-Chain Analysis

## StillJS

### package.json (verbatim, relevant fields)
```json
{
  "name": "@stilljs/core",
  "version": "1.3.14",
  "type": "module",
  "scripts": { "dev": "npx live-server ./ --port 8181", "start": "npx live-server ./ --port 8181" },
  "devDependencies": { "http-server": "^14.1.1" }
}
```

### Findings
- **Runtime dependencies: zero.** This is the one positive — no transitive npm attack surface. `npm audit` would have nothing installed at runtime to flag.
- **No lockfile** (`package-lock.json`/`yarn.lock`/`pnpm-lock.yaml` all absent) → non-reproducible builds; `http-server ^14.1.1` floats. (CWE-1104)
- **Script/dep mismatch:** scripts call `npx live-server` (undeclared → fetched-and-run from the network at dev time), while the declared devDep `http-server` is unused. `npx` of an undeclared package is itself a developer-side remote-execution vector.
- **No npm lifecycle scripts** (`preinstall`/`postinstall`/`prepare`) — confirmed absent (good).
- **No `files` allowlist** in `package.json`; `.npmignore` is minimal → the whole tree (including the `eval`-based renderer) is published.

### Runtime supply-chain surface a scanner won't see
- **Remote `eval` loader:** `setup/components.js:140-142` fetches `cdn.jsdelivr.net/<pkg>@latest/<Cmp>.js` (URL built in `util/route.js:67`) and `eval`s the text. `@latest` is unpinned/mutable; no SRI, no integrity, no allowlist. (CWE-494/CWE-829)
- **Dynamic `<script src>`:** `util/componentUtil.js:66-67`, `BaseComponent.js:802-827`, `components.js:19-28` — no `integrity`/`crossorigin`.
- **Dynamic `import()`:** `util/route.js:9-15` imports the route map file at runtime.

### npm audit / npm ls / npm outdated
Not run against StillJS: with no runtime dependencies and no lockfile there is nothing to resolve; running `npm install` was deliberately avoided (audit rule 13 — do not execute untrusted repository scripts, and the `scripts` fetch `live-server` over the network). Supply-chain risk is intrinsic (remote `eval`), not dependency-tree-borne. **Marked NOT RUN by design.**

## Seiva

- Cargo workspace with a committed `Cargo.lock` (97 KB) — reproducible builds.
- `deny.toml` / `deny.excluded.toml` present (cargo-deny: license + advisory gating).
- The Console Admin crate's only dependencies are `axum`, `delonix-seiva` (default-features off), `seiva-identity`, `serde_json`, `sha2`, `base64` — minimal, no CDN, no remote code.
- `e2e/admin/package.json` declares only `playwright ^1.50.0` (dev/test), driving system Chrome (`PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1`).

## Comparison

| Dimension | StillJS | Seiva |
|---|---|---|
| Runtime deps | 0 | minimal, pinned via Cargo.lock |
| Lockfile | ✗ | ✓ (Cargo.lock) |
| Remote code at runtime | **yes — `eval` of `@latest` CDN** | no (self-contained hashed inline) |
| Advisory/license gating | ✗ | ✓ (cargo-deny) |
| CI | ✗ | ✓ (per workspace tooling) |
