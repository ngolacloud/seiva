# Executive Summary — StillJS Core for Seiva Console Admin

**Decision: REJECT.** StillJS Core must not be integrated into Seiva Console Admin, directly (Option A) or behind an adapter (Option B). Seiva should keep its current secure, server-rendered console (Option C).

**Audited:** `@stilljs/core` v1.3.14 @ commit `3ac755d` · Seiva @ `d1effe9`. Method: direct source reading of both repositories; no repository scripts executed; every finding cites file:line.

---

### 1. What is StillJS?
A single-author, zero-runtime-dependency Vanilla-JavaScript microfrontend framework (~5,900 LOC). Components are ES-module classes with string templates; an engine discovers, loads, and renders them into the DOM.

### 2. What is its actual maturity?
**Pre-1.0 in substance, despite the "1.3.14" label.** No automated tests, no CI (`.github/` absent), no typed or versioned public API, no lockfile, single maintainer. The version number overstates readiness.

### 3. What works well?
- Zero runtime dependencies (small transitive-attack surface).
- A working component/routing model sufficient for demos and small apps.
- No classic prototype-pollution merge sink (verified absent).

### 4. Critical limitations
- **Rendering is `eval`.** Every `${}` interpolation and every `@for`/`@if` directive is executed via `eval()` of template-derived strings (`BaseComponent.js:362`, `helper/template.js:78,145-152`). No Virtual DOM; updates are full `innerHTML` rebuilds. No component teardown — listeners, timers, and workers leak on every navigation. Ordering relies on fixed `setTimeout` delays (race-prone).

### 5. Confirmed security vulnerabilities (blocking)
- **SEC-001 `eval` render engine** (Critical) — requires CSP `unsafe-eval`.
- **SEC-002 DOM-XSS** (Critical) — no HTML-escaping exists anywhere; bound data reaches `innerHTML` raw (`components.js:627`).
- **SEC-003 Remote code execution** (Critical) — fetches `@latest` component source from a CDN and `eval`s it, no SRI/pin/allowlist (`components.js:140-142`, `route.js:67`).
- **SEC-004 Strict CSP impossible** (Critical gate) — needs `unsafe-eval` + `unsafe-inline`; Seiva's `/admin` forbids both.
- **SEC-005/006/007** (High) — inline `onclick=` handlers, `document.write`, and fail-open client-side-only authorization (`components.js:1686`, `router.js:121-192`).

### 6. Main performance concerns
`eval` + full-`innerHTML` rebuild per data change, no diffing/virtualization/batching; document-wide `querySelectorAll` per state mutation; monotonic memory growth (uncleared timers, un-terminated workers, strong global registries). For data-dense admin tables this is pathological. (Not benchmarked — the security gate already decides the matter.)

### 7. Can it be integrated into Seiva?
**No.** Seiva Console Admin runs under a strict, SHA-256-hash-pinned CSP (`default-src 'none'; script-src 'sha256-…'`, no `unsafe-inline`/`unsafe-eval`), escapes every value, uses `addEventListener` only, and enforces authorization server-side. StillJS collides with all of these. **10 of 13 capabilities are architectural CONFLICTs.** An adapter cannot help: StillJS emits `eval`/inline-handlers/remote-`eval` in the browser, below any Seiva abstraction.

### 8. What needs to change (if StillJS were ever reconsidered)?
Remove `eval` from rendering, default-escape all output, eliminate inline `onclick=`, delete the remote-`eval` loader, and add tests/CI. These are a **core rewrite** (`STILLJS_REMEDIATION_ROADMAP.md`), producing effectively a different framework.

### 9. What should NOT change in Seiva?
Keep the hash-pinned CSP, `esc()` output encoding, `addEventListener`-only events, backend-enforced `#[requires]` authorization, fail-closed registry filtering, and the contract-driven (OpenAPI + WireSchema) auto-generated console. These are the assets StillJS would degrade.

### 10. Final recommendation
**REJECT** StillJS. Retain Seiva's existing console. If richer client interactivity is required later, run a fresh engine-selection using the blueprint's gate — no `unsafe-eval`/`unsafe-inline` — which admits CSP-clean options (Web Components/Lit or extending the current vanilla console) and continues to exclude StillJS as-is.

**Weighted scorecard: 13.5 / 100** (Security 0/10 triggers the mandatory gate). Detail in `ARCHITECTURAL_DECISION_RECORD.md`.
