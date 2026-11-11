# EVIDENCE — Security Findings (consolidated register)

Full write-ups with data flows and mitigations are in [`../STILLJS_SECURITY_AUDIT.md`](../STILLJS_SECURITY_AUDIT.md). This register is the quick-reference index with the single strongest evidence line each.

| ID | Title | Sev | CWE | OWASP | Status | Evidence (file:line) |
|---|---|---|---|---|---|---|
| SEC-001 | `eval()` is the render engine | Critical | CWE-95/94 | A03 | CONFIRMED | `BaseComponent.js:362`; `template.js:78,145-152,183-184`; `components.js:141,565` |
| SEC-002 | DOM-XSS: bound data → `innerHTML`, no encoding | Critical | CWE-79 | A03 | CONFIRMED | `components.js:627`; `BaseComponent.js:287-297`; zero escape fn in tree |
| SEC-003 | Remote `@latest` code fetched & `eval`'d, no SRI | Critical | CWE-494/829 | A08 | CONFIRMED | `components.js:140-142`; `route.js:67` |
| SEC-004 | Strict CSP impossible (needs unsafe-eval+inline) | Critical (gate) | CWE-693/1021 | A05 | CONFIRMED | vs `seiva-console-admin/src/lib.rs:452-469` |
| SEC-005 | Inline `onclick=` event binding | High | CWE-79 | A03 | CONFIRMED | `BaseComponent.js:404,407,412,414`; `form.js:41,44` |
| SEC-006 | `document.write()` (post-load; URL-derived strings) | High | CWE-79 | A03 | CONFIRMED | `router.js:122,381,509-511`; `components.js:255` |
| SEC-007 | Client-side-only, fail-open authorization | High | CWE-602/284 | A01 | CONFIRMED | `components.js:1686` (default-allow); `router.js:121-192`; `AppMixin.js:93` |
| SEC-008 | Global `window[...]` writes → clobbering | Medium | CWE-1321 | A03 | CONFIRMED | `template.js:37-146`; `components.js:330,1577` |
| SEC-009 | No lockfile / no CI / no tests | Medium | CWE-1104/778 | A06/A08 | CONFIRMED | `.github` absent; no lockfile; no test script |
| SEC-010 | HTTP: `AbortController` dropped; error returned as response | Low/Med | CWE-754 | — | CONFIRMED | `http.js:5-6,38-44` |
| SEC-011 | `form.js` `insertAdjacentHTML` of field config | Low/Med | CWE-79 | A03 | CONFIRMED | `form.js:56` |
| SEC-012 | `localStorage` store API (token-exposing under XSS) | Low | CWE-922 | A04 | CONFIRMED | `Template.js:44-56` |
| SEC-013 | `Math.random()` for ids / loop-var names | Low | CWE-338 | — | CONFIRMED | `UUIDUtil.js:9`; `template.js:19` |
| SEC-014 | ReDoS-shaped regexes (dev input only) | Med/Low | CWE-1333 | — | THEORETICAL | `constants.js:93,96` |
| SEC-015 | Prototype-pollution merge sink | — | CWE-1321 | — | CONFIRMED ABSENT | only `__proto__` reads (`components.js:1575-1576`) |

## Gate evaluation

Per the master-prompt rule, SEC-001, SEC-002, SEC-003 (Critical, CONFIRMED) and SEC-004 (Critical CSP gate) each independently block adoption. **The security gate fails.**

## Data-flow summary for the top three

1. **SEC-002/XSS:** API/JSON/URL-param/`localStorage` value → component field → unescaped splice into template string (`BaseComponent.js:287-297`) → `innerHTML`/`insertAdjacentHTML` (`components.js:627`, `router.js:281`) → script execution.
2. **SEC-001/eval:** component field in an `@for`/`${}` context → loop body built as JS-source template literal (`template.js:121`) → `eval(content)` (`template.js:151`) → if the value contains `${…}`, `BaseComponent.js:362` re-`eval`s it as code.
3. **SEC-003/RCE:** component reference `npm/…` → `route.js:67` builds `cdn.jsdelivr.net/…@latest/…` → `fetch().text()` → `eval()` (`components.js:141`) → arbitrary JS in the app origin.

## No secrets found
No credentials, tokens, or keys were found committed in the StillJS tree. (Per audit rule 14, none would be reproduced here if found.)
