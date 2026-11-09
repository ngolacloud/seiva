# StillJS Core — Bugs & Engineering Gaps

**Subject:** `@stilljs/core` v1.3.14, commit `3ac755d`. Static review; no fabricated reproductions. Pure code smells are excluded; each item below is a defect or a missing engineering guarantee with code evidence.

> Note: `eval`/`innerHTML`/CSP issues are catalogued in `STILLJS_SECURITY_AUDIT.md` (SEC-001…010) and not repeated here.

---

## BUG-001 — HTTP client drops `AbortController`; returns error as a response

| | |
|---|---|
| **File** | `@still/helper/http.js` |
| **Location** | `HttpRequestOptions` (`:3-7`), `get/post/delete` (`:33-87`) |
| **Evidence** | `signal` field declared (`:6`) but `fetch` is called with only `{method, body, headers}` (`:38-41, 58-62, 78-82`). `catch (error) { return error; }` (`:44,64,85`). |
| **Reproduction** | Pass `{signal}` to `get()`, abort it → request is not cancelled. On network failure, caller does `(await get()).json()` → `TypeError` (error object has no `.json`). |
| **Impact** | No cancellation → stale responses overwrite newer state (race); failures surface as confusing `TypeError`s, not handled errors. |
| **Severity** | Medium |
| **Fix** | Forward `signal` to `fetch`; on failure `throw` or return a typed `{ok:false,error}`. |
| **Test** | Unit: abort a slow request asserts rejection; mock a network error asserts no `TypeError` in caller. |

## BUG-002 — No component destroy: listeners/timers/workers leak on navigation

| | |
|---|---|
| **File** | `@still/setup/components.js`, `@still/component/manager/registror.js` |
| **Location** | `removeOldParts` (`components.js:1371-1418`), `desrtroyCmpInstance` (`registror.js:70-77`) |
| **Evidence** | Teardown clears child `innerHTML` and deletes registry entries only; `removeEventListener`/`terminate`/`unregister` appear **zero** times in `@still/`. `WorkerHelper.traceCmp[id]=cmp` (`BaseComponent.js:1113`) never cleaned. |
| **Reproduction** | Navigate between two views containing `<st-divider>` N times → N pairs of `document` mousemove/mouseup listeners remain (`components.js:1757,1775`). Heap grows monotonically. |
| **Impact** | Memory leak + duplicated handlers + timers firing against destroyed components. |
| **Severity** | High |
| **Fix** | Introduce a real unmount that tracks and releases every listener/timer/worker/subscription per component (see Integration Blueprint Lifecycle Manager). |
| **Test** | Playwright: navigate loop ×50, assert listener count / detached-node count stable. |

## BUG-003 — `setInterval` poller may never clear

| | |
|---|---|
| **File** | `@still/setup/components.js` |
| **Location** | `firstPropagateTimer = setInterval(…,200)` (`:543-554`) |
| **Evidence** | Self-clears only if `firstPropagation[...]` set or `'value' in cmp[field]`; comment `:544`: "Work in the garbage collector for this flag". |
| **Reproduction** | A field that never receives a `.value` wrapper, or a component destroyed before the condition holds → the 200 ms interval runs forever holding a closure over `cmp`. |
| **Impact** | CPU churn + permanent retention of the component. |
| **Severity** | Medium |
| **Fix** | Track the interval id on the instance; clear in unmount; bound retries. |
| **Test** | Unit: destroy a component with an unresolved field; assert the interval is cleared. |

## BUG-004 — Fixed-delay `setTimeout` sequencing → init/render races

| | |
|---|---|
| **File** | `@still/routing/router.js`, `@still/setup/components.js` |
| **Location** | `router.js:151,290,310,321,340-375`; `components.js:257,284,307-323,828-866,1319-1329` |
| **Evidence** | Phase ordering uses `setTimeout(…,10/20/120/200/500)` and 200 ms polling of `getElementById` to detect mount; global mutable `Router.preView/navCounter/clickEvetCntrId` (`router.js:34-37`) clobbered per `goto`. |
| **Reproduction** | Rapid navigation (two `goto`s within 500 ms) or a slow template fetch (`components.js:165`) → a callback from the earlier view runs against DOM replaced by the newer view; or `querySelector` returns `null` and handlers silently no-op. |
| **Impact** | Non-deterministic rendering, dropped handlers, wrong-view mutations. |
| **Severity** | High |
| **Fix** | Replace delay-based sequencing with promises/`MutationObserver` and an apply-time generation counter; serialize navigation. |
| **Test** | Playwright: fast back-and-forth navigation asserts final view is correct and no stale handlers fire. |

## BUG-005 — Global `window[...]` loop variables clobber under re-entrancy

| | |
|---|---|
| **File** | `@still/helper/template.js` |
| **Location** | `:36-42, 100-102, 143-146, 154-160` |
| **Evidence** | `window[variable]`/`window[loopVar]` set to `''` then accumulated, then `eval`'d; data source parked on `window` (`:37-38`) and not always deleted. |
| **Reproduction** | Two `@for` loops whose random ids collide, or a re-entrant render of the same loop, overwrite each other's `window[variable]` mid-`eval`. |
| **Impact** | Corrupted/empty render output; cross-component data bleed. |
| **Severity** | Medium |
| **Fix** | Keep loop state in function/module scope; never on `window`/`global`. |
| **Test** | Unit: render two sibling loops concurrently; assert outputs are independent. |

## BUG-006 — Append-only list diff leaves stale rows

| | |
|---|---|
| **File** | `@still/helper/template.js` |
| **Location** | `updateDomTreeOnAtForDataSourceChange` (`:189-209`) |
| **Evidence** | New/changed items are updated/appended (`:199,205`) but items removed from the data source are not pruned on this path. |
| **Reproduction** | Bind a list, remove an element from the array → the DOM row for the removed item persists. |
| **Impact** | UI shows deleted records (correctness). |
| **Severity** | Medium |
| **Fix** | Reconcile removals (key-set diff) or full-rebuild with keyed replacement. |
| **Test** | Unit: remove middle item, assert DOM node count matches data length. |

## BUG-007 — `document.write()` after load destroys the document

| | |
|---|---|
| **File** | `@still/routing/router.js` |
| **Location** | `:122,138,180,192,381,502,511` |
| **Evidence** | `document.write(authErrorMessage())` / `document.write(err)` executed at runtime, post-DOMContentLoaded. |
| **Reproduction** | Trigger an auth-denied route or a component-not-found → `document.write` re-opens and blanks the document, destroying app state and listeners. |
| **Impact** | App becomes unusable after the error; no recovery. |
| **Severity** | High |
| **Fix** | Render errors into a dedicated container node; never `document.write` post-load. |
| **Test** | Playwright: hit a not-found route, assert the shell survives and shows an inline error. |

---

## Engineering gaps (missing guarantees, not single-line bugs)

| ID | Gap | Evidence | Impact |
|---|---|---|---|
| GAP-01 | **No automated tests** | No `*.test.js`/`*.spec.js`; `package.json` has no `test` script | No regression safety; every change is risky |
| GAP-02 | **No CI** | `.github/` absent | No gate on correctness/security |
| GAP-03 | **No typed/declared public API** | no `.d.ts`, `jsconfig.json` only; API not versioned by contract | Consumers cannot rely on stability; refactors silently break them |
| GAP-04 | **No lockfile; inconsistent dev deps** | `devDependencies: http-server` but scripts use `live-server` | Non-reproducible dev env |
| GAP-05 | **No error boundaries** | `http.js` swallows errors; template `eval` throws to global | Failures are silent or fatal |
| GAP-06 | **No SSR / no CSP story** | engine requires `eval`+inline handlers | Cannot be hardened (see Security) |
| GAP-07 | **No component isolation** | no Shadow DOM/Custom Elements; styles via appended hashed classes; state on `window` | Cross-component interference |

---

## Summary

Seven confirmed defects (two High: no teardown, race-prone `setTimeout` sequencing; one High: `document.write` post-load) and seven structural gaps (no tests, no CI, no typed API). Together they place StillJS below the engineering bar for an enterprise platform even before the security gate is applied.
