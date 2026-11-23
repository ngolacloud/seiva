# ADR 0041 — Native mail: a `Mailer` port with an SMTP adapter and asynchronous send

- Status: Proposed
- Date: 2026-11-23
- Deciders: NgolaCloud
- Follows: ADR 0002 (core ports and in-memory doubles), ADR 0034 (durable job queue + leases)

## Context

Almost every platform has to send transactional email: verify an address, reset a
password, accept an invitation, notify an actor. Seiva has no mail capability at
all today — so the one consumer that needs it, delonix-meet, wires `lettre` by
hand, builds its own templates, and sends **inline inside the request**, which
couples a user-facing latency to a third-party SMTP relay that may be slow or
down. That is exactly the shape that should be a framework primitive: a port the
application codes against, an SMTP adapter behind a feature, and a send that
never blocks the request.

The forces:

- **Port-shaped, adapter-free core.** The application must depend on a `Mailer`
  trait (like every other port, ADR 0002), never on `lettre` directly. The SMTP
  wire, the provider, the credentials are an adapter concern composed at the root.
  A service that sends no mail links no SMTP stack and pays nothing.
- **Sending is not request work.** An SMTP handshake across the network can take
  seconds and can fail transiently. Doing it inside the request ties a login or a
  signup to a relay's health. The send must be **enqueued** and run off the
  request, on the durable job queue (ADR 0034), so the request returns at once and
  the delivery retries on its own schedule.
- **Content is policy, mechanism is horizontal.** The *shape* of sending —
  addressing, headers, MIME, TLS, templating, retry — is true for any platform.
  The *text* of the "verify your address" email, the sender identities, the
  branding are the product's policy. The framework carries the first and injects
  the second.
- **Header injection is a real attack.** A `name` or `subject` built from user
  input that contains `\r\n` can inject extra headers (a hidden `Bcc:`, a forged
  `From:`). The adapter must reject control characters in every header-bound field
  before a message is ever built.
- **Deliverability and transport security are not optional.** TLS to the relay is
  mandatory (STARTTLS or implicit), and the product needs DKIM/SPF/DMARC awareness
  — at minimum a configured signing identity and optional DNS/MX pre-checks — or
  its mail lands in spam. The framework exposes these as config, not as surprises.

## Decision

Add a new workspace member **`seiva-mail`** — data-layer-free, feature-gated,
depending on the `Mailer` port and on `seiva-jobs` (ADR 0034) for async send.

- **`Mailer`** — the port: `async fn send(&self, message: &Mail) -> MailResult<()>`.
  `Mail` is a validated value (envelope `from`/`to`/`cc`/`bcc`, `subject`, a
  text **and** HTML body, optional attachments, custom headers). Construction
  **rejects CR/LF and control characters** in every header-bound field, so a
  header-injection payload never becomes a message.
- **Adapters, composed at the root:**
  - **`SmtpMailer`** (`lettre` feature) — a pooled SMTP transport. **TLS is
    required**: implicit TLS or STARTTLS, never plaintext auth; relay
    authentication (user/password or token) from 12-factor config. A bounded
    **connection pool** amortises the handshake across sends.
  - **`ConsoleMailer`** / **`MemoryMailer`** — the dev/test doubles: render the
    message to stdout or capture it for assertions, so no module needs a live
    relay to be exercised (the ADR 0002 doubles rule).
- **Templating** — a small, product-owned template registry (verification,
  recovery, invitation…) rendering text+HTML from typed context. The engine is
  the mechanism; the templates and copy are the product's policy, overridable per
  tenant. Rendered output is HTML-escaped by default.
- **Asynchronous send via `seiva-jobs`** — the application calls a thin
  `enqueue_mail(message)` that persists a `SendMail` job and returns immediately.
  A worker claims it, renders, and calls the `Mailer`; a transient failure retries
  with bounded backoff and lands in the dead-letter queue after the cap (ADR 0034).
  A synchronous `send` is available for the rare caller that must block.
- **Deliverability awareness** — config for the signing identity (DKIM selector),
  the envelope sender, and **optional DNS/MX pre-checks** before enqueue (a bad
  recipient domain fails fast instead of churning retries). Outbound MX lookups and
  any webhook/callback go through the egress guard (ADR 0045), never a raw socket.
- **Rate limiting** — a per-tenant send budget (reusing `seiva-ratelimit`,
  ADR 0015) caps a compromised or looping caller before it burns the relay's
  reputation.

`tokio` enters with `default-features = false`; no runtime is linked into the
library. `lettre` and the DNS checks are optional features.

## Alternatives considered

- **Call `lettre` directly from each module.** Rejected: it welds the application
  to a transport, re-derives header-safety and templating in every product, and —
  worst — sends inline, coupling request latency to the relay. The port + async
  send is the whole point.
- **Send synchronously inside the request.** Rejected: an SMTP stall becomes a
  user-visible stall and a transient relay failure becomes a lost email. Enqueue +
  retry on the durable queue is strictly better; a synchronous path stays for the
  rare must-block caller.
- **Ship a full templating DSL in the framework.** Rejected: copy, layout and
  branding are policy. A thin registry over a standard engine keeps the
  mechanism-vs-policy line; the product owns the templates.
- **A hosted-provider SDK (SES/SendGrid) as the core.** Rejected as the default:
  SMTP is the sovereign, provider-neutral floor. A provider adapter is a legitimate
  *additional* `Mailer` impl, not the base.

## Consequences

- **Performance.** Sending runs **off the request** on the job queue, so a login
  or signup never waits on SMTP; a pooled SMTP transport amortises the TLS
  handshake across messages; the crate links no runtime and is zero-cost when a
  service sends no mail.
- **Security.** TLS to the relay is mandatory (no plaintext auth); every
  header-bound field rejects CR/LF, closing header injection; bodies are
  HTML-escaped; a per-tenant rate limit caps abuse; MX lookups and callbacks pass
  through the egress/SSRF guard (ADR 0045); credentials come from config and are
  never logged.
- **Resilience.** Delivery is a durable job (ADR 0034) with bounded-backoff retry
  and a dead-letter queue, so a transient relay outage is survived, not lost; send
  timeouts bound a stalled handshake; a secondary relay can be configured as a
  provider fallback.
- delonix-meet deletes its hand-rolled `lettre` wiring and inline sends, keeping
  only its email copy and sender identities as policy; every future platform gets
  transactional mail by depending on one crate.
