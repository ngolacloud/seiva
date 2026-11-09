# StillJS Core — Performance & Memory Review

**Subject:** `@stilljs/core` v1.3.14, commit `3ac755d`. File:line references to the audited source.
**Scope:** rendering cost, re-render strategy, memory stability under mount/unmount, async/race behaviour. No benchmarks were fabricated; where a measured number is absent it is marked **NOT MEASURED**.

---

## 1. Rendering model and its cost

StillJS has **no Virtual DOM and no general node diffing**. Each render runs the full template through ~a dozen `String.replace`/regex passes and then `eval`s interpolations:

- `getBoundTemplate` (`BaseComponent.js:765-794`) runs, per render, over the whole template string: `parseAtForachLogic`, `getBoundState`, `parseAdjustable`, `parseLocalLoader`, `getBoundRender`, `parseStSideComponent`, `getBoundProps`, `getBoundClick`, `getBoundLoop`, `getBoundOnChange`, `getBoundEvt`, `parseDevider`. The authors wrap it in `console.time/timeEnd` (`:767,789`) — an acknowledged hot spot.
- `getBoundState` (`:273-305`) is O(fields × template length): for each field it does `replaceAll('@'+field)` across the entire template.
- Interpolation is `eval` per `${…}` (`BaseComponent.js:362`), plus `eval(`cls.${flag}`)` per `(showIf)`/`(renderIf)` (`:641,706`). Because these are `eval`, V8 cannot JIT/cache them like real functions — they recompile every pass.

**Consequence:** render cost scales with template size × field count × number of interpolations, repeated on every data mutation. For a trivial view this is invisible; for an admin table it is pathological (below).

---

## 2. Large lists / data tables (the admin-critical path)

`@for` (`helper/template.js:85-163`, `runAtForAndIfStatement`) **constructs the loop body as a source string and `eval`s it** (`:151 eval(content)`, `:152 result = eval(variable)`), then writes the whole container via `innerHTML` (`:146`). On any data-source change the entire loop output is rebuilt — O(N) string build + an `eval` compile — even if one row changed.

Two list paths, both full-rebuild:
- **Plain `@for`:** `parseForEachTemplate` sets `fullRerender = true` and concatenates the template for every record on every change (`components.js:867-872`); `parseAndAssigneValue` then replaces the container `innerHTML` wholesale (`:766`). O(N) rebuild per mutation.
- **Component-per-row (`<st-element>`):** `replaceBoundFieldStElement` instantiates a **new component instance per record per pass** (`components.js:800`), runs the full `getBoundTemplate` pipeline per item (`:803,815,823`), registers each in `ComponentRegistror` (`:829`), and schedules per-item `setTimeout`s (`:828,830`). For N rows: N × (full pipeline + registry insert + deferred timers), re-run on data change.

The only node-level "diff" is `updateDomTreeOnAtForDataSourceChange` (`template.js:189-209`): per-item `JSON.stringify` equality keyed on `item.id`, and a changed item is rebuilt with `innerHTML` + `eval` (`handleAtForNewNode`, `components.js:562-570`). Removed items are **not pruned** on this path (append-only, `template.js:201-205`).

**State propagation is document-wide:** every scalar setter triggers `document.querySelectorAll('.state-change-…' / '.listenChangeOn-…')` across the whole document (`components.js:579,588,590`), and text updates use `innerHTML` not `textContent` (`components.js:627`).

### Expected behaviour at the requested scales — **NOT MEASURED** (see gate below)

| Scenario | Expected StillJS behaviour (analytical, from code) |
|---|---|
| 50 components | Workable but each carries deferred `setTimeout` chains and registry entries. |
| 200 components | Noticeable: per-component regex pipeline + timers; global registries grow. |
| 500 components | Degraded: mount storms of `setTimeout(…,10/120/200/500)`, document-wide `querySelectorAll` per mutation. |
| 1,000+ rows in a table | **Pathological:** full `innerHTML` + `eval` rebuild on each data change; per-row component instantiation for `<st-element>`; no virtualization anywhere. |

No list virtualization, windowing, `requestAnimationFrame` batching, or `DocumentFragment` batching exists. Benchmarks were **not** run because the security gate (eval/CSP) already blocks adoption; a perf bench would not change the decision. Marked **NOT MEASURED** per audit rule 20.

---

## 3. Memory stability under mount/unmount — **unstable (leaks confirmed)**

Repeated mount/unmount does **not** produce stable memory. Confirmed leak sources:

- **No teardown contract:** `unload()` (`BaseComponent.js:1284`) only calls `stOnUnload()` (a user no-op) and deletes one registry entry (`registror.js:70-77`). Route change (`removeOldParts`, `components.js:1371-1418`) wipes child `innerHTML` and deletes registry entries whose `navigationId < navCounter`, but releases **no** listeners/timers/workers.
- **`document`-level listeners never removed:** `removeEventListener` appears **zero** times in `@still/`. `<st-divider>` adds `document.addEventListener('mousemove'/'mouseup', …)` on every render with no dedup/removal (`components.js:1757,1775,1817,1821`) — unbounded growth across navigations.
- **Timers never cleared:** `incrementLoadCounter` `setTimeout(…,1000)` per `getTemplate` (`BaseComponent.js:758-762,797`); `firstPropagateTimer = setInterval(…,200)` per field per parse (`components.js:543-554`) — self-clears only under a condition that may never hold (comment at `:544`: "Work in the garbage collector for this flag"). ~62 `setTimeout` vs 3 `clearTimeout` / 2 `clearInterval` total.
- **Workers never terminated:** `terminate`/`unregister` appear **zero** times. `new Worker(import_worker.js)` (`components.js:1597`), app `loadWorker` (`:1639`, accumulates `message` listeners via `BaseComponent.js:1121`), and `navigator.serviceWorker.register(intercept_worker.js)` (`:1623`) that caches every prefetch with no eviction (`intercept_worker.js:22-29`).
- **Strong global registries, no `WeakMap`/`WeakRef`:** `WorkerHelper.traceCmp[id] = cmp` holds a strong reference to **every component ever parsed** (`BaseComponent.js:1113`) and is never cleaned by destroy. Plus `Components.componentPartsMap/subscriptions/parsingTracking/firstPropagation/…`, `BehaviorComponent.currentFormsValidators`, `Router.importedMap/urlParams/parsedRouteMap`, and components pinned on `window[cmp.name]` (`components.js:1577,1579,979,993`). All grow monotonically in a navigating SPA.

**Verdict:** an admin app that navigates repeatedly (the normal usage) leaks listeners, timers, worker message handlers, and detached DOM via the static registries and `window` pins. This is a **maintainability + stability** liability independent of security.

---

## 4. Async execution & races — **no serialization, stale responses can win**

- Ordering is driven entirely by **fixed-delay `setTimeout`** guesses (`parseGetsAndSets` at 10ms, `parseOnChange` at 500ms, `runAfterInit` at 10/20/120/200ms; navigation polling at 200ms — `router.js:151,290,310-322,340-375`). No promise queue or apply-time generation counter exists. If the DOM/data is slower than the guess, `querySelector` returns `null` and handlers silently no-op; if faster/re-entrant (rapid navigation), a callback from an earlier view runs against DOM already replaced by a newer view. `Router.clickEvetCntrId/preView/navCounter` are single global mutable statics (`router.js:34-37`) clobbered on each `goto`.
- **No request cancellation/dedup:** `http.js` declares a `signal` field (`:5`) but **never passes it to `fetch`** (`:38,58,78`). No `AbortController`. A slow in-flight request whose component navigated away still resolves and mutates state via the setter → `propageteChanges`. **Stale responses can overwrite newer state** — confirmed: nothing compares response order or checks component liveness before applying. `catch` returns the error object as a pseudo-response (`:44,64,85`).

---

## 5. Performance verdict

Even setting security aside, StillJS's rendering is **`eval` + full `innerHTML` rebuild with no diffing, no batching, no virtualization**, and its lifecycle **leaks listeners/timers/workers** on every navigation. For a data-dense administrative console (tables, repeated navigation, long-lived sessions) this is the wrong engine on performance and stability grounds alone. Performance is therefore a **secondary** reason to reject, reinforcing the primary (security/CSP) gate.
