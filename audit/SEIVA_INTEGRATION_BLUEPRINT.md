# Seiva Integration Blueprint — and why the adapter does not rescue StillJS

The master prompt asks us to design a Seiva ⇄ StillJS adapter that isolates Seiva from StillJS and preserves replaceability:

```
Seiva Console Admin
    │
Seiva UI Adapter   ← isolation layer
    │
StillJS Core
    │
Native Browser APIs
```

This document evaluates that design honestly and concludes that **the adapter is the right pattern but cannot make StillJS admissible**, because the blocking defects (eval-rendering, inline handlers, remote `eval`) are emitted by StillJS *at the browser layer, below the adapter*. The blueprint is therefore presented as (1) the general isolation pattern Seiva should adopt for **any** future UI engine, and (2) an explicit demonstration of where StillJS breaks it.

> Audit rule honoured: *"Do not implement this architecture unless the audit demonstrates that it is necessary."* The audit demonstrates it is **not** necessary for StillJS, because StillJS is rejected. The blueprint stands as guidance for the engine-selection decision (Option C alternatives).

---

## 1. The adapter boundary (engine-agnostic)

If Seiva ever introduces a client engine, it should sit behind a thin, typed boundary so the engine is replaceable. Proposed layout (new, additive — no change to existing source):

```
crates/seiva-console-admin/           # unchanged: the secure server-rendered console
web/                                   # (only if a client engine is ever adopted)
├── adapter/
│   ├── registry.mjs      # ComponentRegistry: register/resolve/validate/dedupe
│   ├── lifecycle.mjs     # LifecycleManager: mount/update/destroy + teardown ledger
│   ├── render.mjs        # RenderAdapter: ONLY safe sinks (textContent/createElement), escape
│   ├── events.mjs        # EventAdapter: addEventListener only, cleanup ledger
│   ├── state.mjs         # StateAdapter: scoped store, no globals
│   └── security.mjs      # SecurityAdapter: CSP nonce/hash plumbing, trusted-source policy
├── contract/
│   └── contract.mjs      # read-only client of /admin/registry.json + /openapi.json
└── engine/
    └── <engine>.mjs      # the pluggable engine behind the adapter interface
```

### Adapter interfaces (TypeScript-style for clarity)

```ts
interface ComponentRegistry {
  register(name: string, def: ComponentDef): void;   // reject duplicates
  resolve(name: string): ComponentDef | undefined;
  validate(def: ComponentDef): Result;               // contract conformance
}
interface LifecycleManager {
  mount(name: string, host: Element, props: unknown): Handle;
  update(h: Handle, props: unknown): void;
  destroy(h: Handle): void;                           // MUST release every listener/timer/subscription
}
interface RenderAdapter {
  text(node: Element, value: unknown): void;          // textContent only
  el(tag: string, attrs: Record<string,string>): Element; // createElement + setAttribute
  // NO innerHTML of interpolated data; NO eval; NO document.write
}
interface EventAdapter {
  on(node: Element, type: string, handler: EventListener): Off; // addEventListener; returns remover
  // NO inline on* attributes
}
interface StateAdapter { create<T>(init: T): Store<T>; } // module-scoped; never window[...]
interface SecurityAdapter {
  trusts(origin: string): boolean;                    // allowlist for any remote module
  csp(): { scriptSrc: string[]; styleSrc: string[] }; // must not require unsafe-eval/unsafe-inline
}
```

Responsibilities map 1:1 to the prompt's list (Component Registry, Lifecycle Manager, Rendering Adapter, Event Adapter, State Adapter, Security Adapter).

---

## 2. Where StillJS violates each adapter contract

| Adapter contract | StillJS behaviour | Breaks at | Adapter can fix? |
|---|---|---|---|
| `RenderAdapter` = no `innerHTML`/`eval` of data | Renders via `eval` + `innerHTML` with no escape | `BaseComponent.js:362`, `template.js:145-152` | **No** — emitted inside the engine |
| `EventAdapter` = `addEventListener` only | Emits inline `onclick=` | `BaseComponent.js:404-414` | **No** — emitted into the HTML string |
| `SecurityAdapter.csp()` = no unsafe-* | Needs `unsafe-eval` + `unsafe-inline` | §4 security audit | **No** — browser-enforced |
| `SecurityAdapter.trusts()` = allowlist+SRI | Fetches `@latest` CDN text and `eval`s it | `components.js:140-142`, `route.js:67` | **No** — engine owns the fetch |
| `LifecycleManager.destroy()` = full teardown | No teardown; leaks listeners/timers/workers | `components.js:1371-1418` | Partially, but the engine keeps creating them |
| `StateAdapter` = no globals | Writes `window[...]` | `template.js:37-146` | **No** — engine writes globals directly |

**Conclusion:** the adapter cleanly isolates Seiva from an engine's *API*, but it sits **above** the DOM sink. StillJS performs `eval`, inline-handler emission, remote `eval`, and global writes **below** the adapter, directly against `document`/`window`. No code the adapter runs can retroactively make those CSP-compliant. The only "adapter" that would work is one that re-implements rendering itself — at which point StillJS is not being used as a rendering engine at all.

---

## 3. What this means for the decision

- **Option B (StillJS behind an adapter) does not pass the security gate.** The adapter improves API hygiene but leaves SEC-001…SEC-004 fully in force at runtime. Documented in `ARCHITECTURAL_DECISION_RECORD.md`.
- The adapter **pattern** remains the correct way to adopt *any* future engine (e.g. Lit/Web Components, or a small hand-rolled renderer), and this blueprint should be reused in that evaluation. The engine chosen must satisfy `SecurityAdapter.csp()` with no `unsafe-eval`/`unsafe-inline` — which rules StillJS out and admits CSP-clean, Shadow-DOM-isolated options.

No production code is changed by this audit; this blueprint is a design artifact only.
