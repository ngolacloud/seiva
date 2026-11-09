# EVIDENCE — Testing

## StillJS — test inventory
```
$ grep '"test"' still-js/package.json          → no test script
$ find still-js -name '*.test.js' -o -name '*.spec.js'   → no files
$ ls still-js/.github                           → absent (no CI)
```
**StillJS has no automated tests and no CI.** No unit, integration, security, or performance tests exist in the repository. This is a first-order maturity and risk finding (GAP-01/02, SEC-008).

## Static safety checks run during this audit (read-only)

Sink sweep (grep over `@still/**/*.js`) — representative confirmed hits:

| Sink | Representative sites |
|---|---|
| `eval(` | `BaseComponent.js:362,453,641,706,1143`; `template.js:78,145,146,151,152,183,184`; `components.js:141,142,565,566`; `AppMixin.js:29`; `BaseController.js:17` |
| `innerHTML =` | `components.js:99,627,766,944,956`; `template.js:146,183,184,199,204`; `router.js:150`; `Template.js:150,151`; `error.js:33,83,102,127`; `lone.js:60` |
| `insertAdjacentHTML` | `router.js:281,308,415`; `Template.js:102,194`; `form.js:56` |
| `document.write` | `router.js:122,138,180,192,381,502,511`; `components.js:255,1041` |
| dynamic `<script>` | `componentUtil.js:66-67`; `BaseComponent.js:806-814`; `components.js:24-25` |
| `window[...] =` writes | `template.js:37-41,101,143`; `components.js:330,563,979,993,1577,1579` |
| `removeEventListener` / `terminate` / `unregister` | **0 occurrences** (confirms the teardown/leak findings) |
| HTML escape / sanitize / `textContent` for data | **0 encoder occurrences** (confirms the XSS finding) |

No StillJS repository scripts were executed (audit rule 13; the npm scripts fetch `live-server` from the network). A CSP-enforced render test and an XSS-corpus test are *specified* in the remediation roadmap but cannot pass against the current engine (they are the acceptance criteria for P0-1/P0-2).

## Seiva — reusable test stack (for context)

- **Rust unit/router tests:** `#[test]`/`#[tokio::test]` via `tower::ServiceExt::oneshot` — e.g. `crates/seiva-http/src/security.rs:171-305` (asserts security headers on every response, incl. 429), `crates/seiva-console-admin/src/lib.rs:644-698` (registry authz: anonymous sees nothing, only authorized resources listed, conflict → 500).
- **Browser E2E:** Playwright over system Chrome (`e2e/admin/admin.mjs`, `catalog.mjs`; `run.sh` builds `admin-e2e`, serves `127.0.0.1:8099`, drives `/admin`, asserts fail-closed and that passwords are not echoed).

These were **not executed** in this audit (the decision does not require running Seiva's suite); they are documented as the stack any future integration work would reuse. **Marked NOT RUN (out of scope for the rejection decision).**

## Conclusion

The testing evidence reinforces the verdict: the engine proposed for adoption (StillJS) has **zero** test coverage and no CI, while the incumbent (Seiva) has a real Rust + Playwright suite. Adopting StillJS would import an untested, security-hostile rendering engine into a tested, hardened platform.
