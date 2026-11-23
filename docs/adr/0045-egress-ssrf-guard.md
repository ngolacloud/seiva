# ADR 0045 — A guarded outbound HTTP client: egress / SSRF protection for webhooks, mail and OIDC

- Status: Proposed
- Date: 2026-11-23
- Deciders: NgolaCloud
- Follows: ADR 0012 (cloud-native health probes and limits), ADR 0026 (per-route load control), ADR 0033 (webhook delivery engine)

## Context

A platform makes outbound HTTP calls to **addresses it does not control**: a
tenant's webhook URL (ADR 0033), a recipient domain's MX host (ADR 0041), an
identity provider's OIDC discovery and JWKS endpoints (ADR 0040), a user-supplied
"fetch this URL" feature. Every one of those is a **Server-Side Request Forgery**
vector: a tenant who registers a webhook pointing at `http://169.254.169.254/` or
`http://10.0.0.5:6379/` is asking the *server* to reach the cloud metadata service
or an internal Redis on its behalf — from inside the trust boundary, with the
server's network identity. Seiva today has only one passing mention of egress
validation; several new outbound paths (0033, 0040, 0041, 0042) make a single,
correct, shared guard a prerequisite, not a nicety.

The forces:

- **Validate the resolved IP, not the hostname.** Blocking by string ("deny
  `localhost`") is trivially bypassed: `127.0.0.1.nip.io`, a decimal IP, a hostname
  that *resolves* to a private address. The guard must resolve DNS and inspect the
  **actual IP** against the blocked ranges — loopback, private (`10/8`,
  `172.16/12`, `192.168/16`), link-local (`169.254/16`, the metadata IP),
  `fc00::/7`, `::1`, and the rest.
- **DNS rebinding is the subtle attack.** An attacker's domain can pass validation
  (resolves to a public IP), then resolve to `169.254.169.254` on the *second*
  lookup the HTTP client does when it actually connects. The guard must **pin the
  IP it validated** and connect to that exact address, so the IP that was checked
  is the IP that is dialled — no second, unchecked resolution.
- **Redirects re-open the hole.** A public, allowed URL can `302` to
  `http://10.0.0.5/`. Following redirects blindly defeats the whole check. Every
  hop must be **re-validated** with the same rules, and the redirect count bounded.
- **One guard, every caller.** Webhooks, mail MX, OIDC fetch and any product fetch
  must all go through the *same* checked client, or the weakest unguarded call-site
  is the platform's SSRF hole. A single shared, guarded client is the mechanism;
  the per-product allowlists are policy.
- **Guarded ≠ unbounded.** An outbound call is also a resilience surface: it needs
  connect/read timeouts (the ADR 0012/0026 posture, applied outbound) and a
  per-host circuit breaker so one dead dependency cannot stall every worker.

## Decision

Add a **guarded outbound HTTP client** — a small `seiva-egress` surface wrapping
`reqwest`, shared by every outbound path — that validates addresses before and
across the whole request.

- **`EgressPolicy`** — the rules: the blocked IP ranges (loopback, private,
  link-local, unique-local, the metadata `169.254.169.254`/`fd00:ec2::254`), an
  optional per-product **allowlist** (only these hosts/ranges) and **denylist**,
  the permitted schemes (`https` by default), the max redirects, and the timeouts.
  Policy is injected; the mechanism is fixed.
- **Resolve-and-pin dial.** For every connection the guard resolves the host,
  rejects the request if **any** resolved IP is in a blocked range, then **pins**
  the chosen validated IP and connects to it directly — the IP that was checked is
  the IP that is used, closing **DNS rebinding**. A short DNS-resolution cache
  avoids re-resolving on the hot path without widening the rebinding window beyond
  its TTL.
- **Re-validate across redirects.** Automatic redirect-following is replaced by a
  manual loop that runs the **same** resolve-and-pin validation on **each** hop and
  enforces the redirect cap; a hop to a blocked range aborts the whole request.
  Credentials/authorization headers are **not** forwarded across a cross-origin
  redirect.
- **Timeouts and circuit breaking.** Connect and read timeouts bound every call
  (ADR 0012/0026 applied to the outbound direction); a **per-host circuit breaker**
  (reusing `seiva-resilience`, ADR 0033) trips a repeatedly-failing destination so
  it stops consuming workers, and recovers on a half-open probe.
- **The shared client, injected.** `seiva-webhooks` (0033), `seiva-mail` (0041),
  `seiva-notifications` (0042) Web Push, and the OIDC discovery/JWKS fetch (0040)
  all take this client rather than a raw `reqwest::Client`. A product's own
  fetch feature uses it too. There is no supported path to an unguarded outbound
  client in the framework.

`reqwest`/DNS live behind the crate's feature; a service that makes no outbound
calls links none of it.

## Alternatives considered

- **Validate the hostname/URL string only.** Rejected: trivially bypassed by a name
  that resolves to a private IP, by alternate IP encodings, or by a redirect. Only
  inspecting the **resolved IP**, on every hop, is sound.
- **Validate once, then let `reqwest` resolve and connect.** Rejected: that leaves
  a **DNS-rebinding** window — the client's own resolution can differ from the one
  that was checked. Pinning the validated IP is the fix.
- **Follow redirects with `reqwest`'s built-in policy.** Rejected: the built-in
  follower does not re-run an SSRF check per hop, so a `302` to an internal address
  is followed. A manual, re-validating loop is required.
- **A guard per consumer (each crate rolls its own).** Rejected: SSRF is a
  whole-platform property; the one call-site that forgets the check is the breach.
  One shared, reviewed client removes that class of mistake.
- **Rely on network egress firewalling (NetworkPolicy) instead.** Rejected as the
  sole control: network policy is a valuable *additional* layer but is deployment-
  dependent and absent in dev; the application must be safe by itself (defence-in-
  depth), and a `169.254.169.254` block belongs in the client regardless.

## Consequences

- **Performance.** A short DNS-resolution cache and a reused, pooled client keep
  the guard off the hot path's cost; validation is a handful of range comparisons
  per connection; the crate links nothing when a service makes no outbound calls.
- **Security.** Internal and **metadata** IPs (`169.254.169.254`, link-local,
  private, loopback, unique-local) are blocked (**SSRF**); the validated IP is
  **pinned** against **DNS rebinding**; every **redirect** hop is re-validated and
  bounded; internal credentials are **not** forwarded across a cross-origin
  redirect; `https`-only by default.
- **Resilience.** Connect/read **timeouts** bound every outbound call (ADR
  0012/0026); a **per-host circuit breaker** (ADR 0033, `seiva-resilience`) isolates
  a failing destination so one bad dependency cannot exhaust the worker pool; the
  redirect cap bounds a redirect loop.
- Every outbound path in Seiva — webhooks, mail, notifications, OIDC, product fetch
  — becomes SSRF-safe by construction through one injected client, and delonix-meet
  retires its ad-hoc checks for a single reviewed guard.
