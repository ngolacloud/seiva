# StillJS Remediation Roadmap (upstream guidance)

This roadmap is addressed to the **StillJS maintainers** (or to anyone considering a hard fork). It is **not** a Seiva work item — Seiva's decision is to not adopt StillJS (`ARCHITECTURAL_DECISION_RECORD.md`). It documents what would have to change for StillJS to become admissible in a strict-CSP enterprise console. The P0 items constitute a core rewrite.

Complexity: **S** (hours) · **M** (days) · **L** (1–2 weeks) · **XL** (weeks+, cross-cutting).

---

## P0 — Critical (security blockers; without these, adoption is impossible)

### P0-1 — Remove `eval` from the render path
- **Objective:** render without `eval`/`new Function` so the engine runs under `script-src` with no `unsafe-eval`.
- **Files:** `@still/component/super/BaseComponent.js:362`; `@still/helper/template.js:78,145,146,151,152,183,184`; `@still/setup/components.js:565,566`; `@still/component/super/AppMixin.js:29`; `service/BaseController.js:17`; `BaseComponent.js:453,641,706,1143`.
- **Approach:** replace string-compiled templates with a parsed AST / precompiled render functions that build DOM via `createElement`/`textContent`; resolve `${}` via a whitelisted, non-`eval` expression evaluator; implement `@for`/`@if` as structural directives over nodes, not `eval`'d source.
- **Acceptance:** app renders under `Content-Security-Policy: script-src 'self'` (no `unsafe-eval`); zero `eval`/`new Function` in `@still/`.
- **Tests:** CSP-enforced render test; grep gate in CI forbidding `eval(`.
- **Complexity:** **XL**

### P0-2 — Default output encoding at every interpolation sink
- **Objective:** eliminate DOM-XSS; escape all data by default.
- **Files:** data sinks in `BaseComponent.js:287-297`, `setup/components.js:99,627,766,944,956`, `helper/template.js:146,199,204`, `helper/form.js:56`, `routing/router.js:150,281,308`, `component/super/Template.js:150-151`.
- **Approach:** contextual escaping (HTML-text / attribute / URL); use `textContent` for text; opt-in raw HTML only through an explicit `unsafeHtml` directive that routes through a sanitizer.
- **Acceptance:** a bound value `"<img src=x onerror=…>"` never executes.
- **Tests:** XSS corpus rendered through list/interp/attribute paths.
- **Complexity:** **L**

### P0-3 — Eliminate inline `onclick=` event binding
- **Objective:** no inline event handlers (CSP `unsafe-inline`).
- **Files:** `@still/component/super/BaseComponent.js:375-417,1076-1079`; `helper/form.js:41,44`; `setup/constants.js:11`.
- **Approach:** compile `(click)=…` to `addEventListener` with a handler reference bound at mount; return removers for teardown (P1-2).
- **Acceptance:** rendered HTML contains no `on*=` attributes; handlers work.
- **Complexity:** **L**

### P0-4 — Remove remote `eval` component loader / harden remote loading
- **Objective:** no arbitrary remote code execution.
- **Files:** `@still/setup/components.js:140-142`; `@still/util/route.js:62-71`; dynamic `<script src>` in `util/componentUtil.js:63-75`, `BaseComponent.js:802-827`, `components.js:19-28`.
- **Approach:** delete the `eval` loader; if remote components are required, load via `import()` of exact-version-pinned, SRI-hashed, origin-allowlisted modules under `'strict-dynamic'`. Never `eval` network text. Never `@latest`.
- **Acceptance:** no network text is `eval`'d; remote modules carry SRI.
- **Complexity:** **M**

### P0-5 — Stop `document.write`; encode URL-derived error strings
- **Files:** `@still/routing/router.js:122,138,180,192,381,502,511`; `setup/components.js:255,1041`.
- **Approach:** render into a container node; escape `pathValue`/`cmpName`.
- **Complexity:** **S**

---

## P1 — High (safe enterprise integration)

### P1-1 — Backend-authz-first guidance; demote route guards to UX
- Document that `isPublic`/whitelist/`isAuthN` are presentation-only; fix the fail-open default (`components.js:1686`) to fail-closed. **Complexity: S.**

### P1-2 — Real component teardown
- Add `destroy()` that releases every listener/timer/subscription/worker; track them per instance; call on route change. Files: `components.js:1371-1418`, `registror.js:70-77`, divider listeners `components.js:1757,1775,1817,1821`. **Complexity: L.**

### P1-3 — Deterministic lifecycle (remove `setTimeout` sequencing)
- Replace fixed-delay ordering with promises/`MutationObserver`; add an apply-time generation counter; serialize navigation. Files: `router.js:151,290,310-375`, `components.js:257,284,307-323`. **Complexity: L.**

### P1-4 — HTTP client: wire `AbortController`, dedup, typed errors
- Forward `signal` to `fetch`; cancel on unmount; guard against stale responses; stop returning the error object as a response. File: `helper/http.js`. **Complexity: M.**

### P1-5 — Remove global `window[...]` state
- Keep loop/render state in module scope; never `window[name]`. Files: `helper/template.js:37-146`, `components.js:330,563,979,993,1577,1579`. **Complexity: M.**

---

## P2 — Medium (performance, maintainability, DX)

- **P2-1** Node-level reconciliation / keyed diffing for lists (prune removals — `template.js:189-209`); batch with `DocumentFragment`/`requestAnimationFrame`. **L.**
- **P2-2** List virtualization for large tables. **L.**
- **P2-3** `crypto.randomUUID()` instead of `Math.random()` ids (`UUIDUtil.js:9`, `template.js:19`). **S.**
- **P2-4** Simplify ReDoS-shaped regexes (`constants.js:93,96`). **S.**
- **P2-5** Terminate/unregister workers; bound the service-worker cache (`components.js:1597,1623,1639`). **M.**

## P3 — Low (hygiene)

- **P3-1** Add tests + CI (currently none); grep gates for `eval(`/`innerHTML`/`on*=`. **M.**
- **P3-2** Commit a lockfile; declare `live-server`; add a `files` allowlist to `package.json`. **S.**
- **P3-3** Ship TypeScript types / a documented, versioned public API. **M.**
- **P3-4** Error boundaries around render and async. **M.**

---

## Reality check

P0-1 through P0-4 replace StillJS's rendering engine, event binding, and module loader — the majority of its differentiating code. Completing them yields a **different framework** with StillJS's name. For Seiva, the pragmatic path (Option C) is to not depend on StillJS and, if interactivity is later needed, adopt a CSP-clean engine behind the adapter blueprint.
