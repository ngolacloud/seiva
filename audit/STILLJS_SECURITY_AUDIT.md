# StillJS Core — Security Audit

**Subject:** `@stilljs/core` v1.3.14, commit `3ac755d`
**Frameworks used:** OWASP Top 10 (2021), OWASP ASVS v4, OWASP DOM-XSS Prevention, CWE.
**Method:** Static source review. No repository scripts executed. Every finding cites file:line and quotes code. Findings are marked **CONFIRMED** (code path proven) or **THEORETICAL** (requires an unverified condition).

> **Headline:** StillJS cannot run under Seiva Console Admin's security model. Its core rendering engine depends on `eval()` (requires CSP `unsafe-eval`), its event binding emits inline `onclick=` attributes (requires `unsafe-inline`), and it loads code via dynamic `<script src>` and **`eval` of remote network-fetched text**. In addition, data-bound values reach `innerHTML` **without output encoding**, reintroducing DOM-XSS into a console that is currently XSS-free. These are architectural, not incidental.

---

## Severity summary

| ID | Title | Severity | CWE | OWASP |
|---|---|---|---|---|
| SEC-001 | `eval()` is the core template/render engine | **Critical** | CWE-95 / CWE-94 | A03 Injection |
| SEC-002 | DOM-XSS: data-bound values reach `innerHTML` without encoding | **Critical** | CWE-79 | A03 Injection |
| SEC-003 | Remote code fetched over network and `eval`'d (no SRI/pinning) | **Critical** | CWE-494 / CWE-829 | A08 Software & Data Integrity |
| SEC-004 | Strict CSP is impossible (`unsafe-eval` + `unsafe-inline` required) | **Critical (gate)** | CWE-1021 | A05 Misconfiguration |
| SEC-005 | Event binding emits inline `onclick=` attributes | **High** | CWE-79 | A03 Injection |
| SEC-006 | `document.write()` used for auth errors and routing | **High** | CWE-79 | A03 |
| SEC-007 | Client-side-only authorization (`isPublic`/whitelist) | **High** | CWE-602 | A01 Broken Access Control |
| SEC-008 | Global-namespace writes (`window[...]`) — clobbering/leak | **Medium** | CWE-1321 (adjacent) | A03 |
| SEC-009 | No lockfile / no SRI / no CI — supply-chain integrity gap | **Medium** | CWE-1104 | A06 / A08 |
| SEC-010 | `http.js` drops `AbortController` signal; returns error as response | **Low/Medium** | CWE-754 | — |

---

## SEC-001 — `eval()` is the core template/render engine — **CONFIRMED, Critical**

**CWE-95 (eval injection) / CWE-94 (code injection). OWASP A03.**

`eval()` is not a corner case in StillJS; it is the rendering primitive. Enumerated sinks:

- `@still/component/super/BaseComponent.js:362` — every `${expr}` in a template:
  ```js
  template = template.replace(/\${(...)}/gi, (_, exp) => eval(`${exp}`));
  ```
- `@still/helper/template.js:78` — `@if` condition: `const isTrue = eval(`${exp}`);`
- `@still/helper/template.js:145,146` — `eval(content); ... .innerHTML = eval(variable);`
- `@still/helper/template.js:151,152` — `eval(content); result = eval(variable);` (runs the `@for` body)
- `@still/helper/template.js:183,184` — reactive `@if` re-evaluation.
- `@still/component/super/AppMixin.js:29` — `return eval(`StillAppSetup.config.props.${path}`);`
- `@still/component/super/service/BaseController.js:17` — `instance = eval(`new ${type}()`);`
- `@still/component/super/BaseComponent.js:453,641,706,1143` — class resolution / render-flag evaluation / scoped `eval(scope)`.
- `@still/setup/components.js:141,142` — `eval` of remote component source (see SEC-003).

**Impact:** `eval` cannot be used under a CSP without `'unsafe-eval'` (SEC-004). Any attacker-influenced expression at any of these sites is arbitrary code execution. Even with trusted templates, it makes the framework fundamentally incompatible with a hardened console.

**Mitigation:** Eliminating `eval` requires rewriting the template compiler and directive engine (interpolation, `@for`, `@if`, binding, controller resolution) to a parsed-AST or DOM-building model. This is a core rewrite, not a patch.

**Regression test:** Load any StillJS app under `Content-Security-Policy: script-src 'self'` (no `unsafe-eval`) and assert the page renders — it will not.

---

## SEC-002 — DOM-XSS: data-bound values reach `innerHTML` without output encoding — **CONFIRMED, Critical**

**CWE-79. OWASP A03.**

There is **no HTML-escaping function anywhere in the data-binding path.** Bound values are rendered by building template-literal source in `helper/template.js` (`content += variable + '+=`' + lnContnt… + '`;'`, `template.js:121`), `eval`'ing it, and assigning the result to `innerHTML` (`template.js:146`, `:199`, `:204`). The `{item.field}` / `{bind}` tokens are turned into `${…}` inside those literals (`template.js:121`, `BaseComponent.js:362`) and concatenated into HTML **verbatim**.

**Data flow (source → sink):**
1. **Source:** a component field bound in a `@for` list or `{expr}` interpolation — typically API/JSON data (`fetch` → component field → template).
2. **Transform:** `eval`'d template literal concatenates the value into an HTML string.
3. **Sink:** `document.getElementById(containerId).innerHTML = eval(variable)` (`template.js:146`), or `node.innerHTML = Components.handleAtForNewNode(...)` (`template.js:199,204`).

**Exploitation:** A data record whose field value is `<img src=x onerror=alert(document.cookie)>` executes on render. In an admin console listing tenant-controlled records, this is **stored DOM-XSS driven by ordinary application data** — exactly the class Seiva's `esc()` currently prevents.

**Secondary channel:** `data-stvalue='{JSON.stringify(iterVar)}'` (`template.js:116`) embeds JSON into a single-quoted attribute; a value containing `'` breaks the attribute, and values containing backtick/`${}` can perturb the eval'd template literal (eval-injection adjacent, THEORETICAL for code-exec but CONFIRMED for attribute breakout).

**Mitigation:** contextual output encoding at every interpolation sink (HTML-text vs attribute vs URL). Impossible to bolt on cleanly because the sink is `eval`'d string concatenation, not a structured node API.

**Regression test:** render a list where a field = `"<img src=x onerror=window.__xss=1>"`; assert `window.__xss` is never set.

---

## SEC-003 — Remote code fetched and `eval`'d, no integrity — **CONFIRMED, Critical**

**CWE-494 (download of code without integrity check) / CWE-829. OWASP A08.**

`@still/setup/components.js:140-142`:
```js
const remoteCmp = await (await fetch(npmRoute.cdn+'.js')).text();
eval(`${remoteCmp.replace(/import[\s\S]*?\;/g,'').replace(/export[\s]*?class/g,'class')}; window.${npmRoute.cmp} = new ${npmRoute.cmp}()`);
```
A "npm/CDN" component is fetched as **text** and executed via `eval`. The CDN URL is built with an **unpinned `@latest`** tag — `route.js:67`: `cdn: \`https://cdn.jsdelivr.net/${…}@latest/${cmpName}\`` — so a newly-published malicious version is picked up automatically. There is **no Subresource Integrity, no hash pinning, no origin allowlist, no version lock**. A compromised or substituted CDN response is immediate arbitrary code execution in the app origin. Dynamic `<script src=path>` injection (`util/componentUtil.js:66-67`, `BaseComponent.js:806-814`, `setup/components.js:24-25`) has the same no-integrity property.

**Mitigation:** remove remote `eval`; if remote components are required, use `import()` of SRI-pinned, origin-allowlisted modules under `'strict-dynamic'` — again a redesign.

---

## SEC-004 — Strict CSP is impossible — **CONFIRMED, Critical (adoption gate)**

**CWE-1021. OWASP A05.** This is the decisive integration finding.

Seiva Console Admin serves (`crates/seiva-console-admin/src/lib.rs:452-469`):
```
default-src 'none'; script-src 'sha256-…'; style-src 'sha256-…';
connect-src 'self'; img-src 'self' data:; base-uri 'none';
form-action 'none'; frame-ancestors 'none'
```
No `unsafe-inline`, no `unsafe-eval`. StillJS violates this policy on **every** axis:

| StillJS mechanism | Evidence | CSP directive violated | Would require |
|---|---|---|---|
| `eval()` rendering | SEC-001 | `script-src` | `'unsafe-eval'` |
| inline `onclick=` | `BaseComponent.js:404,407,412,414` | `script-src` | `'unsafe-inline'` (or `'unsafe-hashes'`) |
| dynamic `<script src>` | `componentUtil.js:66`, `BaseComponent.js:814` | `script-src` | origin allowlist or `'strict-dynamic'` |
| remote `eval` | `components.js:141` | `script-src` | `'unsafe-eval'` |
| `document.write` | `router.js:122,381,502,511` | (breaks after parse) | — |

**Conclusion:** adopting StillJS forces Seiva to weaken `/admin` to at least `script-src 'unsafe-eval' 'unsafe-inline'` — a wholesale regression of the console's security posture, re-opening the entire XSS/injection surface the current design closed. **No adapter can prevent this**, because `eval`/inline-handlers are emitted by StillJS's engine at runtime inside the browser, below any Seiva-side abstraction.

---

## SEC-005 — Inline `onclick=` event binding — **CONFIRMED, High**

`@still/component/super/BaseComponent.js:375-417` (`getBoundClick`) rewrites `(click)="method(args)"` into **`onclick="ClassPath.method(args)"`** inline attributes (`:404, :407, :412, :414`). Inline event handlers are a CSP `unsafe-inline` dependency and a classic injection surface if any arg is data-derived. Mitigation: `addEventListener` with handler references (as Seiva's console already does).

---

## SEC-006 — `document.write()` for auth/routing — **CONFIRMED, High**

`@still/routing/router.js:122,138,180,192,381,502,511` call `document.write(...)`. After the document has loaded, `document.write` **erases the entire document** and re-opens it, destroying all state and listeners, and is a known XSS sink when its argument is data-derived (`authErrorMessage()` is static here, so XSS is THEORETICAL, but the API misuse is CONFIRMED). Mitigation: render into a container node, never `document.write` post-load.

---

## SEC-007 — Client-side-only authorization — **CONFIRMED, High**

**CWE-602 (client-side enforcement of server-side security). OWASP A01.**

`@still/routing/router.js:121-192` gates views on `instance.isPublic`, `Components.obj().isInWhiteList(instance)`, and `AppTemplate.get().isAuthN()`, writing `authErrorMessage()` on denial. This is **presentation-layer visibility, not authorization** — trivially bypassed by a user who loads the component directly or edits client state. Worse, the whitelist **fails open**: `setup/components.js:1686` — `if(!isInBlackList && !isInWhiteList) isInWhiteList = true;` — anything not explicitly blacklisted is treated as allowed. `isAuthN()` is a client-set in-memory boolean (`AppMixin.js:93`); setting it in the console bypasses the guard. The component class, template, and its data-fetching code are shipped to the browser regardless.

**This is not automatically a vulnerability in Seiva** *provided* every operation remains enforced server-side. Seiva already does this: `seiva-console-admin` performs **no** client authz and relies on the module's `#[seiva::controller]` guards; its `can` flags are explicitly presentational (`lib.rs:507-529`). **The risk is regression**: StillJS's model actively encourages treating `isPublic`/whitelist as the gate. **Rule to preserve:** frontend checks must never replace Seiva's backend authorization.

---

## SEC-008 — Global-namespace writes — **CONFIRMED, Medium**

`@still/helper/template.js:37-41, 101-102, 143, 146, 156, 159` assign to `window[loopVar]` / `window[variable]` (and `global[...]` off-DOM). Loop variable names are derived from `Math.random()` (`template.js:19`) and the data-source identifier. This pollutes the global scope, risks **name clobbering** (incl. DOM-clobbering if an attacker can place elements whose `id` collides with a global the engine reads), and leaks references (memory). `Math.random` ids are also unsuitable for anything security-relevant. Mitigation: module-scoped maps, never `window[...]`.

---

## SEC-009 — Supply-chain integrity gaps — **CONFIRMED, Medium**

`package.json` ships **no lockfile**, **no CI** (`.github/` absent), **no tests**, and declares `devDependencies: http-server` while `scripts` invoke `live-server` (undeclared). Combined with SEC-003 (remote `eval`, no SRI) the integrity story is weak. The published surface (`.npmignore` ships `@still/**`) has **no runtime dependencies**, which is the one positive — the attack surface is the framework's own `eval`, not a transitive dep tree.

---

## SEC-010 — HTTP client defects — **CONFIRMED, Low/Medium**

`@still/helper/http.js`: `HttpRequestOptions` declares a `signal` field (`:6`) but `get()` passes only `{method, headers}` (`:38-41`) — **`AbortController` is dropped**, so requests cannot be cancelled and **stale responses can overwrite newer state** (race). `catch` returns the `error` object (`:44,64,85`) as if it were a `Response`, so callers calling `.json()` on failure throw. No CSRF-token handling; `getURL` passes absolute `http(s)` URLs straight through (`:25`) — client-side fetch to arbitrary origins (CORS-limited). Mitigation: wire `signal`; throw/return a typed error.

---

## What StillJS does *not* expose

- No transitive dependency vulnerabilities (no runtime deps) — `npm audit` would find nothing to upgrade because there is nothing installed at runtime.
- No obvious secret material committed in the audited tree.
- No catastrophic-backtracking regex *confirmed* (the large regexes in `template.js`/`BaseComponent.js` are broad but bounded by template size, not attacker-length input in the common path) — marked **NOT FULLY VERIFIED**; recommend a focused ReDoS pass if adoption were ever reconsidered.

---

## Security gate decision

Per the master-prompt gate ("a critical exploitable vulnerability or an unresolved high-risk architectural weakness can block adoption regardless of total score"), SEC-001–SEC-004 are **blocking**. StillJS fails the security gate for integration into Seiva Console Admin.
