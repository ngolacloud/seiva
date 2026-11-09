# ADR — StillJS Core as the Seiva Console Admin rendering engine

- **Status:** Decided — **REJECT** (do not adopt StillJS for Seiva Console Admin)
- **Date:** 2026-11-05
- **Deciders:** Architecture / Security / Platform audit team
- **Subjects:** `@stilljs/core` v1.3.14 @ `3ac755d` · Seiva @ `d1effe9`
- **Security gate:** per the master-prompt rule, a critical exploitable vulnerability or unresolved high-risk architectural weakness blocks adoption regardless of score. **The gate is triggered** (SEC-001…SEC-004).

---

## Context

Seiva's Console Admin is a dependency-free, server-rendered page under a strict, SHA-256-hash-pinned CSP (`default-src 'none'; script-src 'sha256-…'`, no `unsafe-eval`/`unsafe-inline`), with `esc()` output encoding, `addEventListener`-only events, and fully backend-enforced authorization (`seiva-console-admin/src/lib.rs`, `seiva-http/src/{security,guard}.rs`, `seiva-macros`). The question: can StillJS become its component/rendering engine without compromising that posture?

## Options

- **Option A — Adopt StillJS directly.** Rejected: its core render path is `eval` + unescaped `innerHTML`; incompatible with the CSP and reintroduces DOM-XSS.
- **Option B — Adopt StillJS behind a Seiva adapter.** Rejected: the blocking defects (eval, inline `onclick=`, remote `eval`) are emitted by StillJS in the browser, below any adapter; the adapter cannot make them CSP-compliant (`SEIVA_INTEGRATION_BLUEPRINT.md §2`). Fails the same gate.
- **Option C — Do not adopt StillJS.** **Chosen.** Seiva keeps its secure, contract-driven, server-rendered console. If richer client interactivity is ever needed, select a CSP-clean engine (Web Components/Lit or a small hand-rolled renderer) behind the adapter pattern from the blueprint.

## Decision

**REJECT StillJS for integration into Seiva Console Admin (Options A and B).** Proceed with **Option C**.

## Rationale (evidence)

1. **CSP is non-negotiable and StillJS cannot meet it.** StillJS requires `unsafe-eval` (rendering is `eval`: `BaseComponent.js:362`, `helper/template.js:78,145-152,183-184`) and `unsafe-inline` (inline `onclick=`: `BaseComponent.js:404-414`). Adopting it forces `/admin` to downgrade to `script-src 'unsafe-eval' 'unsafe-inline'`, discarding the console's primary XSS defence. (SEC-001, SEC-004, SEC-005.)
2. **It reintroduces DOM-XSS.** No HTML-escape function exists anywhere in StillJS; bound data reaches `innerHTML` raw (`components.js:627`, `BaseComponent.js:287-297`). Seiva currently escapes every value (`esc()`). (SEC-002.)
3. **Remote code execution by design.** StillJS fetches `@latest` component source from jsDelivr and `eval`s it with no SRI/pin/allowlist (`components.js:140-142`, `route.js:67`). (SEC-003.)
4. **Client-side-only, fail-open authorization.** `isAuthN`/whitelist guard that defaults to allow (`components.js:1686`) and uses `document.write` (`router.js:121-192`). Seiva's backend authz is sound; StillJS actively encourages the anti-pattern. (SEC-007.)
5. **Engineering immaturity.** No tests, no CI, no typed/stable API, no component teardown (leaks listeners/timers/workers), race-prone `setTimeout` sequencing, `eval`+full-`innerHTML` rendering with no virtualization. (Bugs & Performance reviews.)
6. **No upside to offset the risk.** There is no incumbent JS framework to replace; Seiva's console already auto-generates from the contract with zero per-module frontend code and superior security. StillJS would subtract capability (security, tests) while adding a hard dependency.

## Decision scorecard

Scores 0–10. "Measured" = grounded in read code; perf is analytical (NOT MEASURED) and marked provisional.

| Category | Weight | Score | Weighted | Basis |
|---|---:|---:|---:|---|
| Architecture | 15% | 2 | 0.30 | eval-string engine, no isolation, no teardown, global state |
| **Security** | 25% | **0** | **0.00** | eval render, no encoding, remote eval, client authz — gate fails |
| Performance | 15% | 2 | 0.30 | full `innerHTML`+eval rebuilds, leaks; no virtualization (provisional) |
| Maintainability | 10% | 2 | 0.20 | no types, informal code, single maintainer |
| Component system | 10% | 3 | 0.30 | works for demos; 3 incoherent load paths; no Shadow DOM |
| Testing & quality | 10% | 0 | 0.00 | no tests, no CI |
| Seiva compatibility | 10% | 1 | 0.10 | 10/13 capabilities are CONFLICT |
| Documentation & maturity | 5% | 3 | 0.15 | README/CHANGELOG exist; API unversioned; v1.x overstated |
| **Total** | 100% | — | **1.35 / 10 → 13.5 / 100** | |

**13.5 / 100 → "Below 50: Reject for production integration."** Independently, the **security gate alone** mandates rejection regardless of score.

## Consequences

- **Positive:** Seiva keeps its strict CSP, output encoding, backend authz, tests/CI, and zero-dependency admin. No new client-side attack surface, no untested dependency, no lock-in.
- **Negative / follow-up:** if richer interactivity is later required, run a fresh engine-selection using the blueprint's `SecurityAdapter.csp()` gate (no `unsafe-eval`/`unsafe-inline`). Candidates to evaluate: native Web Components / Lit (Shadow DOM isolation, CSP-clean), or extending the existing vanilla console. StillJS is not a candidate unless it eliminates `eval`, inline handlers, and remote `eval` from its core — effectively a rewrite.
- **For the StillJS maintainers (optional, upstream):** see `STILLJS_REMEDIATION_ROADMAP.md` — the P0 items would make StillJS a different framework; they are offered as upstream guidance, not as a Seiva work item.

## Final verdict

**REJECT.** The evidence is in `STILLJS_SECURITY_AUDIT.md` (SEC-001…SEC-004 blocking), `STILLJS_ARCHITECTURE_REVIEW.md`, `STILLJS_PERFORMANCE_REVIEW.md`, `STILLJS_BUGS_AND_GAPS.md`, and `SEIVA_COMPATIBILITY_MATRIX.md`.
