# StillJS Core — Architecture Review

**Subject:** `@stilljs/core` v1.3.14 — https://github.com/still-js/core
**Audited commit:** `3ac755d0aa736d3b724da8e3556ba0b254fa9d14` ("update the changelog and package.json")
**Size:** ~5,916 LOC of ES-module Vanilla JavaScript under `@still/`
**Method:** Direct source reading (no execution of repository scripts). File:line references are to the audited commit.

---

## 1. What StillJS is

StillJS is a zero-runtime-dependency, Vanilla-JavaScript microfrontend framework. A component is an ES-module class extending `ViewComponent`/`BaseComponent`; it carries a `template` string (inline or loaded from an `.html` file), declares typed fields, and is placed in the page through custom tags (`<still-component>`, `<st-element>`, component class tags). Applications are bootstrapped from `@still/index.js` into `#stillUiPlaceholder`, configured by `config/app-setup.js` and a static `config/route.map.js`.

It positions itself (package keywords) as a "Microfrontend / Vanilla UI JavaScript Framework". It is a **single-author hobby/early-stage project** (one `author` in `package.json`, no organisation, no CI).

### Module map (role of each unit)

| Path | LOC | Role |
|---|---:|---|
| `@still/setup/components.js` | 1914 | The engine: component discovery, dynamic loading (`import()` + `<script src>` injection + **remote `eval`**), registration, mounting, `@for` node handling. |
| `@still/component/super/BaseComponent.js` | 1286 | Component base: template parsing, `${}` interpolation (**via `eval`**), `(click)` binding (**to inline `onclick=`**), field/prop binding, lifecycle plumbing. |
| `@still/routing/router.js` | 582 | Hash/path routing, component swap via `innerHTML`/`insertAdjacentHTML`, **client-side auth guard** (`isPublic`/whitelist → `document.write`). |
| `@still/component/super/BehaviorComponent.js` | 446 | Form validation state, change-event wiring, behavioural mixins. |
| `@still/helper/template.js` | 247 | `@for`/`@if` directive compiler — builds JS source strings and **`eval`s them**, renders via `innerHTML`. |
| `@still/component/super/Template.js` | 203 | App shell template, toast/notification UI (`insertAdjacentHTML`, `innerHTML`), `localStorage` template cache. |
| `@still/setup/constants.js` | 98 | Constants, `authErrorMessage()`, no-template message. |
| `@still/setup/error.js` | 136 | Error UI fragments rendered via `innerHTML`. |
| `@still/helper/http.js` | 108 | `StillHTTPClient` — thin `fetch` wrapper. |
| `@still/helper/form.js` | 76 | Form helpers, validation message injection (`insertAdjacentHTML`). |
| `@still/component/super/AppMixin.js` | 123 | App-level mixin; config property access **via `eval`**. |
| `@still/component/manager/*.js` | ~200 | Web-worker offload helpers (`load_worker`, `import_worker`, `intercept_worker`), `registror.js` (component registry Map), `timer.js`. |
| `@still/util/*.js` | ~165 | `componentUtil.js` (dynamic `<script>` import), `route.js` (`import()` of route file), `UUIDUtil.js` (`Math.random`-based ids). |
| `@still/lone.js` | 82 | "Lone" standalone component embedding (`outerHTML` reparse + `innerHTML`). |

---

## 2. Component model

- **Creation / inheritance:** class extends `ViewComponent` (→ `BaseComponent` → `BehaviorComponent`). Composition is by referencing a child component's class/tag inside a parent template; children are discovered by scanning the rendered template for known tags (`setup/components.js`).
- **Registration / discovery:** two mechanisms coexist — (a) a static `stillRoutesMap` in `config/route.map.js` giving `path`/`url` per component, and (b) runtime scanning of the DOM/template. A global `ComponentRegistror` (`manager/registror.js`) holds instances in a `Map`.
- **Instantiation:** `setup/components.js` resolves a component to a filesystem/URL path and loads it one of three ways:
  1. `await import(`${cmpPath}.js`)` — ES dynamic import (local components), `components.js:150`.
  2. Dynamic `<script src>` injection — `util/componentUtil.js:66-67`, `BaseComponent.js:806-814`, `setup/components.js:24-25`.
  3. **Remote fetch + `eval`** for "npm/CDN" components — `setup/components.js:140-142`:
     ```js
     const remoteCmp = await (await fetch(npmRoute.cdn+'.js')).text();
     eval(`${remoteCmp.replace(/import[\s\S]*?\;/g,'').replace(/export[\s]*?class/g,'class')}; window.${npmRoute.cmp} = new ${npmRoute.cmp}()`);
     newInstance = eval(`${npmRoute.cmp}`);
     ```
- **Isolation:** none. No Shadow DOM, no Web Components / Custom Elements registry, no style scoping beyond hashed class names appended to templates. Components share the global `window` namespace (see §5).
- **Destruction:** there is **no first-class unmount/destroy contract**. Component swap on routing replaces container `innerHTML` (`router.js:150`, `:281`, `:308`), orphaning the previous subtree and any listeners/timers bound to it (see Bugs & Performance reviews).

**Assessment:** The component model is a bespoke string-template + global-registry design, not aligned with the platform's Custom Elements / Shadow DOM primitives. Three divergent loading paths (one of which is `eval` of remote text) is an incoherent, security-hostile abstraction.

---

## 3. Rendering strategy (verified — *not* a Virtual DOM)

StillJS has **no Virtual DOM and no reconciliation**. Rendering is **string templating compiled through `eval`, then injected as `innerHTML`/`insertAdjacentHTML`**:

- `${expr}` interpolation: `BaseComponent.js:362`
  ```js
  template = template.replace(/\${(...)}/gi, (_, exp) => eval(`${exp}`));
  ```
  Every interpolation point is a JavaScript `eval` of the extracted expression.
- `@for`/`@if` directives (`helper/template.js`): the template is rewritten into **JavaScript source strings** that are then `eval`'d to produce HTML, and the HTML is written with `innerHTML`:
  - `template.js:78` — `const isTrue = eval(`${exp}`)` (`@if` condition).
  - `template.js:145-146` — `eval(content); document.getElementById(containerId).innerHTML = eval(variable);`
  - `template.js:151-152` — `eval(content)` (runs the loop), `result = eval(variable)`.
  - `template.js:183-184` — reactive `@if`: `if(eval(cmp['stOnChangeAtIf'][id]) === true) ...innerHTML = content`.
- Incremental update for list items (`template.js:189-209`, `updateDomTreeOnAtForDataSourceChange`) is a **per-item `innerHTML` rebuild** keyed by `data-stvalue` JSON comparison — not true diffing; each changed row has its `innerHTML` regenerated via `Components.handleAtForNewNode`.
- Full component swaps replace a container's entire `innerHTML` (`router.js:150`) or `insertAdjacentHTML('afterbegin', …)`.

**Output encoding:** none in the data path. Bound values flow into `innerHTML` through `eval`'d template literals **without HTML escaping** — contrast the Seiva Console Admin, which escapes every value with `esc()`. (Detailed data-flow in the Security Audit.)

**Assessment:** The rendering engine is the single most consequential design decision, and it is built on `eval` + unescaped `innerHTML`. This is simultaneously the **performance** liability (full re-parse/re-eval per render) and the **security** liability (DOM-XSS + CSP incompatibility).

---

## 4. Lifecycle

Hooks observed: `stBeforeInit` (pre-render, added in this release), `onRender`, and various internal `afterInit`/`parseOnChange` phases sequenced largely with **`setTimeout`** (see router.js `:151, :246, :283, :290, :310, :311, :321, :322, :340, :351, :464` and BaseComponent `:451, :759, :863`).

Missing lifecycle guarantees:
- **No deterministic mount barrier** — ordering relies on fixed `setTimeout(…, 100/200/500)` delays, not promises/observers. This is a race-condition generator (see Bugs review).
- **No unmount/destroy hook** — nothing releases listeners, timers, workers, or registry entries when a component leaves the DOM.
- **No error boundary** — `http.js` swallows errors by returning the error object as if it were a `Response`; template `eval` failures throw into global scope.

---

## 5. State, events, module system, routing (summary; detail in dedicated reviews)

- **State/reactivity:** field-level pseudo-reactivity via `.value` wrappers and setters, but the `@for` compiler writes loop variables and results into **global `window[...]`** (`helper/template.js:37-41, 101-102, 143, 146, 156`), polluting and clobbering the global namespace and leaking references.
- **Events:** `(click)` handlers are rewritten to **inline `onclick=` attributes** (`BaseComponent.js:375-417`); change handlers use `.onchange =` assignment (`BaseComponent.js:430`). No delegation; re-render duplicates handlers; no `removeEventListener`.
- **Module system:** ES modules + dynamic `import()` + `<script src>` injection + remote `eval`. No integrity/SRI, no version pinning (no lockfile), public API not versioned by contract.
- **Routing:** present (`router.js`) — path/alias routing, history usage, **client-side-only auth guard** (`isPublic`/`isInWhiteList`/`isAuthN`) that calls `document.write(authErrorMessage())` on denial, destroying the document. Navigation does not clean up the outgoing view.

---

## 6. Architectural boundaries & coupling

- **Circular/implicit coupling:** `helper/template.js` imports `config/app-setup.js` (app config) — the "template helper" is coupled to application configuration. `BaseComponent`/`components.js`/`router.js` form a tightly interwoven trio with shared global singletons (`$still`, `window[...]`, `ComponentRegistror`, `WorkerHelper`).
- **Global singletons:** `$still`, `StillAppSetup`, `window.STILL_HOME_PREXIF/LOCAL`, and arbitrary `window[loopVar]` entries — the framework's state lives on `window`.
- **Missing contracts:** no TypeScript types, no declared public-API surface, no stability guarantees; `jsconfig.json` only. `.npmignore` ships the whole tree.

---

## 7. Maturity signals

- **No automated tests** anywhere in the repo; `package.json` has **no `test` script** (only `dev`/`start` via `live-server`).
- **No CI** (`.github/` absent).
- **No lockfile**; `devDependencies` list `http-server`, scripts invoke `live-server` (not declared) — inconsistent.
- Single maintainer; commit messages are informal; version `1.3.14` despite the above.

**The "1.x" version number overstates maturity.** By the standard enterprise criteria (tests, CI, typed/stable API, security posture) this is pre-1.0 in substance.

---

## 8. Architecture verdict

StillJS is an **`eval`-centric string-templating engine** with a global-namespace state model, no Virtual DOM, no component isolation, no destroy lifecycle, and no tests/CI. Its two load-bearing mechanisms — `eval` for rendering and inline `onclick=`/dynamic `<script>`/remote-`eval` for loading — are precisely the mechanisms a hardened enterprise console must forbid. See `STILLJS_SECURITY_AUDIT.md` and `ARCHITECTURAL_DECISION_RECORD.md`.
