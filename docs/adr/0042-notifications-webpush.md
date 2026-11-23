# ADR 0042 — Native notifications: a multi-channel hub with in-app, Web Push and email, fed by the event bus

- Status: Proposed
- Date: 2026-11-23
- Deciders: NgolaCloud
- Follows: ADR 0021 (in-process event bus and choreography), ADR 0031 (native real-time), ADR 0034 (durable job queue + leases), ADR 0041 (native mail)

## Context

"Something happened — tell the user." That sentence recurs in every platform: an
invitation arrives, a meeting is about to start, a mention is made, an export is
ready. The pieces to deliver it now exist in Seiva separately — the event bus
(ADR 0021) says *what happened*, real-time (ADR 0031) can push in-app, mail
(ADR 0041) can email — but nothing ties them into a single **notification** with
a store, per-user preferences, and delivery across channels. delonix-meet builds
this by hand per feature, so a "you were invited" is wired three different ways.
The hub that fans one domain event out to the right channels, respecting the
user's choices, is horizontal and belongs in the framework.

The forces:

- **One notification, many channels.** A notification is a first-class record
  (type, actor, subject, payload, read/unread), not an ephemeral push. It is
  stored for the in-app inbox **and** delivered out-of-band to the channels the
  user opted into: in-app (via ADR 0031), Web Push, email (via ADR 0041). The
  record is the source of truth; the channels are deliveries of it.
- **Fed by the bus, not by call-sites.** A module that places an order should not
  know how notifications are delivered. It publishes its domain event (ADR 0021);
  a notification subscriber maps the event to a notification and fans it out. This
  keeps the producer ignorant of delivery — the choreography principle.
- **User preferences are policy, delivery is mechanism.** *Whether* a user wants
  email for mentions is the user's choice; *how* an email is sent is the framework's
  job. The hub stores preferences per user per notification-type and honours them;
  the product defines the types and their defaults.
- **Web Push is a protocol, not a vibe.** It has a real spec: a subscription
  (`endpoint` + `p256dh` + `auth` keys) obtained in the browser, a payload
  **encrypted** to those keys (`aes128gcm`, RFC 8291), and a **VAPID** JWT
  (RFC 8292) signed with the server's key identifying the application server. The
  framework must implement this correctly, once — a hand-rolled encryption is a
  security bug waiting to happen.
- **Fan-out is off the request.** Encrypting a payload per subscription and POSTing
  to each push service is I/O that must not block the event's producer. It runs on
  the job queue (ADR 0034), batched, with retry.

## Decision

Add a new workspace member **`seiva-notifications`** — data-layer-free (a
`NotificationStore` port), feature-gated, orchestrating the existing primitives.

- **The notification model** — `Notification` (id, tenant, recipient, type,
  actor, subject, `data`, `created_at`, `read_at`) behind a `NotificationStore`
  port (in-memory double; DORM/sqlx adapter per ADR 0030). The store is the in-app
  inbox: list, mark-read, unread-count, all **tenant-scoped**.
- **The fan-out pipeline** — a `seiva-events` subscriber (ADR 0021) receives a
  domain event, a product-registered mapper turns it into a `Notification`, the
  store persists it, and the hub enqueues one delivery job **per opted-in channel**
  on `seiva-jobs` (ADR 0034). Producers stay ignorant of channels.
- **Channels, each an adapter:**
  - **In-app** — publishes the stored notification to the recipient's real-time
    `Topic` (ADR 0031), so an open client sees it live; the store covers offline
    clients.
  - **Web Push** — a `PushSubscriptionStore` holds `{endpoint, p256dh, auth}` per
    user/device, tenant-scoped. Delivery **encrypts** the payload with
    `aes128gcm`/RFC 8291 to the subscription keys and signs a **VAPID** JWT
    (RFC 8292). A `410 Gone`/`404` from the push service **purges** the dead
    subscription.
  - **Email** — renders and enqueues via `seiva-mail` (ADR 0041).
- **Preferences** — a `NotificationPreferences` store, per user per type per
  channel, with product-defined defaults; the fan-out consults it before enqueuing.
  An unsubscribed channel is simply not enqueued.
- **Policy the product keeps** — the notification *types*, their templates, their
  default preferences, and the event→notification mappers. The framework carries
  the store, the pipeline, the Web Push crypto, and the channel adapters.

All outbound Web Push POSTs go through the egress/SSRF guard (ADR 0045). `tokio`
enters with `default-features = false`; the Web Push and email channels are
optional features, so a service with only an in-app inbox links neither.

## Alternatives considered

- **Push directly from each call-site over real-time.** Rejected: it couples every
  producer to delivery, skips the stored inbox (offline users lose the event), and
  re-implements preferences everywhere. The bus-fed hub with a store is the point.
- **A single "notify" channel (in-app only).** Rejected: users expect email and
  browser push for things they are not looking at. Multi-channel with per-user
  preferences is the baseline expectation.
- **Hand-roll the Web Push encryption per product.** Rejected outright: RFC 8291
  `aes128gcm` and VAPID are easy to get subtly wrong, and a wrong payload cipher is
  a security defect. Implement it once, reviewed once.
- **Treat a notification as a fire-and-forget event.** Rejected: a notification is
  a durable, read/unread record the user comes back to; modelling it as an event
  loses the inbox and the read-state.

## Consequences

- **Performance.** Fan-out is **asynchronous** on the job queue, so the event's
  producer never waits on push/email I/O; deliveries are batched; the in-app
  channel rides the existing real-time hub at near-zero marginal cost; the Web Push
  and email channels are optional features.
- **Security.** Web Push payloads are **encrypted** (RFC 8291 `aes128gcm`) and
  authenticated with **VAPID** (RFC 8292); subscriptions and notifications are
  **tenant-scoped**; channels are strictly **opt-in** per user; outbound POSTs pass
  through the egress/SSRF guard (ADR 0045); no payload is logged.
- **Resilience.** Delivery is a durable job (ADR 0034) with bounded-backoff retry;
  a `410 Gone` from a push service **purges** the expired subscription so dead
  endpoints self-heal; multi-channel means one channel's outage still reaches the
  user through another and always through the stored inbox.
- delonix-meet collapses its per-feature notification wiring into event mappers +
  preferences, and gains browser push for free; every future platform gets a
  notification hub by depending on one crate.
