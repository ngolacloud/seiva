# StillJS ↔ Seiva Console Admin — Compatibility Matrix

**Legend:** FULL = compatible as-is · PARTIAL = usable with caveats · ADAPTER = only via an isolation layer · CONFLICT = architectural collision · NOT SUPPORTED = absent in StillJS · NOT VERIFIED = not exercised.

The decisive column is **CSP**: Seiva's `/admin` runs under `default-src 'none'; script-src 'sha256-…'` with **no `unsafe-eval`/`unsafe-inline`** (`seiva-console-admin/src/lib.rs:452-469`). Any capability requiring `eval` or inline handlers is a hard **CONFLICT**, not something an adapter can mediate — the violation happens in the browser, below any Seiva abstraction.

| Capability | StillJS mechanism (evidence) | Seiva today | Compatibility | Required action |
|---|---|---|---|---|
| **Components** | String-template classes, global registry, 3 load paths incl. remote `eval` (`components.js:140-142`) | None (server-rendered) | **CONFLICT** | Would replace secure vanilla with eval-engine |
| **Rendering** | `eval` + full `innerHTML`, no VDOM (`BaseComponent.js:362`, `template.js:145-152`) | `esc()` + `createElement`/`innerHTML` of escaped strings | **CONFLICT** (CSP + XSS) | None viable without engine rewrite |
| **Lifecycle** | Hooks exist; **no real unmount**; `setTimeout`-sequenced (`components.js:1371-1418`) | N/A | **NOT SUPPORTED** (teardown) | Build Lifecycle Manager (if ever adopted) |
| **Events** | Inline `onclick=` (`BaseComponent.js:404-414`) + `.onchange=`; no delegation; no removal | `addEventListener` only | **CONFLICT** (CSP `unsafe-inline`) | None viable |
| **State** | `__defineGetter/Setter` + global `window[...]` (`template.js:37-146`) | Local JS state objects | **ADAPTER** (if eval solved) + CONFLICT on globals | — |
| **Routing** | Client router w/ `document.write` guard, client-only authz (`router.js:121-192`) | Server routes; single-page admin | **CONFLICT** (document.write / client authz) | Keep server routing |
| **Forms** | `(click)`/`onchange` inline binding, no output encoding (`form.js:56`) | Schema-driven forms from WireSchema, escaped | **CONFLICT** (XSS/CSP) | Seiva's forms already superior |
| **Tables** | Full `innerHTML`+`eval` rebuild per change, no virtualization (`components.js:867`) | Server-paginated table, escaped cells, optimistic concurrency | **CONFLICT** + perf regression | Seiva's table already superior |
| **Modals** | — (toast/notification via `insertAdjacentHTML`, `Template.js:150-194`) | Not needed / inline | **NOT SUPPORTED** | — |
| **Notifications** | Toast UI with raw `innerHTML` (`Template.js:150-151`) | `rowmsg` text via `textContent` | **PARTIAL** (XSS risk) | — |
| **Lazy loading** | Dynamic `import()` + `<script src>` + remote `eval` (no SRI) | Single hashed inline bundle | **CONFLICT** (CSP/integrity) | — |
| **Error handling** | `http.js` swallows errors; template `eval` throws global; `document.write(err)` | RFC-9457 Problem responses; inline `out` text | **CONFLICT** | — |
| **Security** | `eval`, no encoding, client authz, remote `eval` | Strict CSP, escape, backend authz | **CONFLICT** (blocking gate) | Reject |
| **Testing** | **None** (no tests, no CI) | Rust `oneshot` + Playwright | **NOT SUPPORTED** (StillJS side) | Would import untested engine |
| **Module system** | ESM + dynamic import + remote eval | Rust crate, one hashed asset | **ADAPTER** (ESM only; remote eval must go) | — |
| **CSP / strict CSP** | Requires `unsafe-eval` + `unsafe-inline` (§4 security) | `sha256`-pinned, none unsafe | **CONFLICT** (hard gate) | None viable |

## Classification summary

- **CONFLICT (10):** Rendering, Events, Routing, Forms, Tables, Lazy loading, Error handling, Security, CSP, Components — i.e. **every load-bearing capability**.
- **NOT SUPPORTED (3):** Lifecycle teardown, Modals, Testing.
- **ADAPTER / PARTIAL (remainder):** only the peripheral pieces (state plumbing, ESM, notifications) could be mediated — and only *after* the CONFLICT items were solved, which requires rewriting StillJS's core.

## Why "REQUIRES ADAPTER" is not an escape hatch here

An adapter can normalise an API surface; it **cannot** change what the wrapped engine emits into the DOM at runtime. StillJS's `eval` rendering and inline `onclick=` are produced *inside the browser by StillJS itself*, after any Seiva adapter has handed off. Therefore the CSP/XSS CONFLICTs are **not adapter-addressable**. The only ways to satisfy Seiva's CSP are: (a) a StillJS core rewrite removing `eval`/inline-handlers/remote-`eval` (a different framework in practice), or (b) not adopting StillJS. See `SEIVA_INTEGRATION_BLUEPRINT.md` for why the adapter option (Option B) still fails the gate.
