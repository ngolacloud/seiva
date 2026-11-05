# Sending email

> **Status: a planned port.** This page describes the intended shape; email is not
> a first-class port in the framework yet.

Transactional email (a welcome message, a password-reset link, a receipt) is an
**outbound side effect** — and Seiva already has the right pattern for those.

## The intended shape: an outbound port + the outbox

Email should be an application-layer **port** (a trait the application calls,
e.g. `Mailer::send`), implemented by an adapter in `infrastructure`. For
reliability, a transactional email belongs on the **transactional outbox**, like
domain events: enqueue the "send welcome email" in the same transaction as the
user-creation it follows, and let a worker deliver it — so an email is never sent
for a change that rolled back, and never lost if the provider is briefly down. See
[messaging and events](messaging-and-events.md) and
[background workers](background-workers.md).

## Today

Until a `Mailer` port ships, send email from an `infrastructure` adapter your
module injects (an SMTP or provider-API client), ideally triggered off the outbox
relay rather than inline in the request. Treat the provider credentials as
[configuration](configuration.md) (environment, never baked in).

## Related

- [Messaging and events](messaging-and-events.md) · [Background workers](background-workers.md)

---

*Part of the [Seiva documentation](../index.md).*
