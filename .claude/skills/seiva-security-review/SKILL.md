---
name: seiva-security-review
description: Run the OWASP API Security Top 10 (2023) checklist against a Seiva change. Use before any PR that touches seiva-http, seiva-auth, seiva-webhooks or the CI security gate, or when asked to security-review the framework.
---

# Seiva security review — OWASP API Security Top 10 (2023)

Walk the table below against the **current code** (read the files; never answer
from memory). For each item say: the control that exists today (`file:line`),
what the PR under review changes, and what is still missing. Finish with a
verdict: does the PR leave any item **without a control**? If yes, block.

Also run the dependency gate: `cargo deny check` (default features) must be
clean, and the all-features advisory pass must not add a NEW advisory beyond the
tracked broker-client debt in `docs/progress.md`.

| # | Risk | Control today (verify it still holds) |
| --- | --- | --- |
| API1 | Broken Object Level Authorization | `Principal.tenant` + `Principal::require_owns` (`seiva-http/src/guard.rs`). Object-level checks live where the resource is loaded (handler/application), not the middleware. CHECK: a new resource-returning path calls `require_owns`. |
| API2 | Broken Authentication | Argon2id PHC (`seiva-auth`); `LoginThrottle` lockout. GAP still open: no reference auth middleware that *populates* `Principal` (JWT/session). CHECK: any new login path uses the throttle; secrets never logged or returned. |
| API3 | Broken Object Property Level Authorization | DTO per facade (explicit wire contract). CHECK: no response DTO leaks a secret field (password/hash/token). |
| API4 | Unrestricted Resource Consumption | `DefaultBodyLimit` + `rate_limit::RateLimiter` (`seiva-http`). CHECK: a new router still mounts the rate limiter; expensive endpoints are not unbounded. |
| API5 | Broken Function Level Authorization | `permission_guard` denies by default (`seiva-http/src/guard.rs`); empty `requires` passes. CHECK: every guarded facade method mounts the guard; no endpoint is left "naked". |
| API6 | Unrestricted Access to Sensitive Business Flows | Webhooks: HMAC-SHA256, constant-time compare, replay window (`seiva-webhooks`). CHECK: a new sensitive flow has an equivalent guard. |
| API7 | Server Side Request Forgery | No outbound HTTP client in the framework yet. When one lands, require a destination allowlist. |
| API8 | Security Misconfiguration | `security::security_headers` (CSP/HSTS/nosniff/X-Frame/Referrer) + CORS by explicit allowlist (`seiva-http/src/security.rs`). CHECK: headers still applied; CORS default stays same-origin (empty allowlist → no layer). |
| API9 | Improper Inventory Management | Link-time facade discovery (`#[seiva::facade]` → `registered_facades()`); runtime `/openapi.json` lists every mounted facade. CHECK: a new facade appears in the served document. |
| API10 | Unsafe Consumption of APIs | Inbound HMAC verified. Same allowlist as API7 when an outbound client exists. |

## How to run

1. `git diff` the change; list the files it touches under the three security crates.
2. For each affected row, read the cited file and confirm the control is intact
   (not weakened, not bypassed by a new path).
3. Any NEW behaviour (a CORS origin, a guarded route, a rate limit) must come
   with its own test — flag it if it does not.
4. Run `cargo deny check`; report advisories.
5. Output the table with today's status + the PR delta + a GO/NO-GO verdict.
