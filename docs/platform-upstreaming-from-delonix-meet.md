# Upstreaming platform capabilities into Seiva — the delonix-meet driver

- Status: Working document (drives ADRs 0031–0045)
- Date: 2026-11-23
- Authors: NgolaCloud

## Why this document exists

Seiva aims to be the Rust framework for building **cloud-native compute
platforms** — Django-like structure, FastAPI-like async ergonomics, 12-factor
throughout. The best way to grow a framework well is to let a real, demanding
consumer pull capabilities up into it, rather than designing primitives in a
vacuum.

[**delonix-meet**](https://github.com/angolardevops/delonix-meet) is that
consumer: a self-hosted, sovereign video-conferencing platform in Rust
(~114 500 LoC of backend, 120 Postgres tables, 348 routes, a single
`delonix-server` binary). It already builds — by hand — much of what a service
framework should provide. This document records **what should be lifted into
Seiva as native capability** (so delonix-meet, and every future platform, just
reuses it), and **what must stay in the product** because it is domain-specific.

The analysis below is evidence-based: it was produced by reading both code bases
(Seiva at `fix/rust-audit-tail-security`; delonix-meet at `develop`) and
measuring, not estimating.

## The boundary principle

One rule decides every line below:

> **The framework provides the horizontal *mechanism*; the product injects the
> *policy* of its domain.**

A capability belongs in Seiva when it is true for *any* platform (a connection
hub, a retrying delivery engine, an object-store port, a token issuer) and does
not leak the domain. It stays in the product when it only makes sense for
"meetings with media" (RTP forwarding, a signalling message schema, a recorder's
ffmpeg pipeline).

## Coverage map — the delonix-meet backend, split

Measured by LoC over `server/src` (114 491 total):

| Plane | Weight | Nature | Framework fit |
|---|---|---|---|
| **A — Control / application** | **~60–65% (~74k LoC)** | Multi-tenant CRUD, auth, RBAC/SoD, orgs/branches/seats, calendar, webhooks, API keys, SCIM, OAuth, analytics, audit, config | **Seiva does this well.** delonix-meet is itself extracting this into DDD crates (core/domain/store) with ports and fail-closed RLS — the very model Seiva formalises |
| **B — Media / real-time** | **~35–40% (~40k LoC)** | SFU (webrtc-rs), DTLS/SRTP, RTP fan-out, simulcast, low-latency signalling, recording, PSTN bridge | **Not framework work.** Needs forks of webrtc-rs, room-sharded in-memory state, a pinned crypto provider. Imported as a crate, driven by events — never rewritten in a framework |

The boundary already exists in their code and is **measured**: `sfu.rs`,
`sfu_e2e.rs`, `phone_bridge/*` carry **zero SQL** and **zero coupling to
`AppState`** — ADR-0004 there calls the SFU "an isolated bounded context by
nature". What interleaves the two planes is a thin seam (room-tokens, presence,
the recorder's library writes) — and that seam is itself mostly generic.

## A — Capabilities to upstream into Seiva (what delonix-meet will reuse)

Each row becomes an ADR (0031–0045). "State" is relative to Seiva today.

### Tier 1 — high reuse, unblocks the use case, low risk

| ADR | Capability | Mechanism Seiva gains | Policy the product keeps | State |
|---|---|---|---|---|
| 0031 | **`seiva-realtime`** (WebSocket + SSE) | WS upgrade, short-lived connection-token auth, connection registry, topic/channel fan-out, heartbeat, backpressure, **Redis pub/sub multi-pod bridge**, presence | the signalling protocol (offer/answer/ice, reactions, hand, breakouts) | **new** — closes blocker #1 and the framework's #1 strategic gap |
| 0032 | **`seiva-storage`** (object store) | `ObjectStore` port + S3/MinIO and local-fs adapters, presigned URLs, multipart | what is stored (recordings, exports) | **new** |
| 0033 | **Webhook delivery engine** (extends `seiva-webhooks`) | durable delivery queue + retry (reuses `seiva-resilience`) + dead-letter + `deliveries` ledger + the signing that already exists | which events fire | **partial — only the engine is missing** |
| 0034 | **`seiva-jobs`** (durable queue + leases) | persistent queue, lease/claim, retry, scheduling, atop the existing `seiva-worker` loop | the work (transcription, ffmpeg, export, mail) | **partial — the generic queue is missing** |

### Tier 2 — enterprise identity (horizontal, more opinionated)

| ADR | Capability | Mechanism | State |
|---|---|---|---|
| 0035 | **Refresh tokens** (rotating, revocable) | issue/rotate/revoke + hashed store, reuse detection | **new** |
| 0036 | **`seiva-auth-mfa`** (TOTP) | TOTP ceremony, backup codes, step-up | **new** |
| 0037 | **`seiva-auth-webauthn`** (passkeys) | WebAuthn ceremonies (webauthn-rs) + credential-store port | **new** |
| 0038 | **`seiva-oauth`** (Authorization Server 2.1) | authz-code + PKCE + token endpoint + client registry (Seiva today only *verifies* JWTs) | **new** |
| 0039 | **`seiva-scim`** (provisioning 2.0) | Users/Groups endpoints, filtering, idempotent sync | **new** |
| 0040 | **OIDC SSO login flow** (extends `seiva-identity`) | discovery + **JWKS rotation** + authorization-code (today JWKS rotation is delegated to the composition root) | **partial** |

### Tier 3 — platform services (horizontal, lower urgency)

| ADR | Capability | Mechanism | State |
|---|---|---|---|
| 0041 | **`seiva-mail`** | SMTP (lettre) + templating + DNS checks, port-shaped | **new** |
| 0042 | **`seiva-notifications`** + **Web Push** | in-app channel + Web Push (VAPID), fed by the event bus | **new** |
| 0043 | **Append-only audit** (extends `AuditSink`) | immutable, hash-chained audit log implementation | **partial — port exists** |
| 0044 | **Native Postgres RLS** (data-layer option) | RLS as a fail-closed backstop under the adapter; their `tenant_tx`/`SET LOCAL` is the reference | **new** (Seiva scopes in-app, not via RLS) |
| 0045 | **Egress / SSRF guard** | outbound address validation (block private/metadata ranges) for webhooks and fetch | **partial — one mention** |

## B — Frontier (mechanism up, semantics stay)

These look product-specific but their skeleton is generic:

- **Signalling** → the connection hub + pub/sub + presence go up (`seiva-realtime`); the message protocol stays.
- **Recording** → the job queue + object store go up; the RTP→IVF/OGG + ffmpeg + E2EE logic stays.
- **Webhooks** → delivery + retry + signing go up; the event catalogue stays.
- **Auth** → refresh/MFA/passkeys/oauth/scim go up; RBAC/SoD/approvals/org-entry rules stay (they are the meet's business rules).

## C — Stays in the product (not framework work)

The media plane, intentionally out:

> SFU webrtc-rs · RTP forwarding · DTLS/SRTP · simulcast q/h/f + RTCP-driven
> selection · anti-glare negotiation · recorder (depacketise + ffmpeg) · E2EE
> Insertable Streams · PSTN bridge (SIP/ESL/FreeSWITCH, SRTP legs, G.711↔Opus) ·
> RTMP broadcast · dubbing/MLS · room-sharded affinity · pinned `CryptoProvider`.

## Priority (value ÷ risk)

1. **`seiva-realtime` (0031)** — biggest prize: unblocks the use case *and* closes
   the framework's strategic gap (no platform framework can lack first-class
   WebSocket/SSE).
2. **`seiva-storage` (0032) + webhook engine (0033) + `seiva-jobs` (0034)** — high
   reuse, low risk, and three of them reuse code Seiva already has
   (`resilience`, `webhooks`, `worker`).
3. **Enterprise identity (0035→0040)** — large but 100% horizontal; makes Seiva
   enterprise-ready.
4. **Tier 3 (0041→0045)** — as the pilot demands it.

## Cross-cutting NFR mandate for every ADR

Every capability below is specified against three axes, not just functionality:

- **Performance** — zero-cost-when-unused (opt-in layers, no runtime linked into
  core crates), bounded allocations, no N+1 over the data layer, async all the
  way down.
- **Security** — fail-closed by default (ADR 0019), deny-by-default authz, secrets
  never logged, defence-in-depth (e.g. RLS under app scoping), OWASP API Top 10.
- **Resilience** — timeouts and backpressure on every outbound path (ADR 0026),
  retries with bounded backoff + circuit breaking (`seiva-resilience`),
  at-least-once with idempotency (ADR 0011), graceful shutdown/draining (ADR 0012).

## References

- ADRs 0031–0045 (this series).
- Existing: ADR 0002 (ports), 0004 (DORM gateway), 0006 (identity), 0011
  (idempotency), 0012 (health & limits), 0013 (admin audit), 0019 (fail-closed),
  0021 (event bus), 0026 (load control), 0028 (worker safety), 0030 (pluggable
  data layer).
- Evidence base: Seiva `@fix/rust-audit-tail-security`; delonix-meet `@develop`.
