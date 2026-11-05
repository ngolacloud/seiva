# ADR 0013 — Persistent audit of administrative mutations

- Status: Accepted
- Date: 2026-11-05
- Deciders: NgolaCloud
- Follows: ADR 0002 (ports), ADR 0006 (identity), ADR 0008/0009 (admin)

## Context

The brief (§6) requires a **persistent audit** of administrative mutations:
actor, tenant, operation, object, result, correlation id and **redacted**
differences — and it must **not** store the full request body with its secrets.
The admin console calls the normal guarded API routes (ADR 0008/0009), so "admin
mutations" are, at the transport, mutating requests by an authenticated principal.

Two forces shape the design. First, like idempotency, the base must not depend on
a specific database — the audit sink is a port. Second, unlike idempotency, the
audit must capture *attempts* including denials, and it is driven from the
transport (where a middleware sees every mutation and its outcome) rather than
from inside one use case's transaction — so it cannot rely on the request's
ambient DORM session.

## Decision

**An `AuditSink` port records `AuditEntry` values; an `audit` middleware records
every mutating request's metadata; the DORM-backed sink persists to `seiva_audit`
on its own session. Secrets are redacted before an entry is ever recorded.**

- `seiva_ports::AuditEntry` (actor, tenant, operation, object, outcome,
  request_id, redacted `details`) + `AuditSink::record`. `AuditEntry::redacted`
  masks every sensitive `details` value, and `is_sensitive_field` (passwords,
  hashes, tokens, keys, secrets, credentials, authorization, cookies — broad,
  case-insensitive) is the one definition both the transport and a handler use.
- `seiva_http::audit` middleware: for `POST`/`PUT`/`PATCH`/`DELETE` it records the
  actor (from the authn `Principal`), the operation (`METHOD path`), the outcome
  status and the correlation id — **metadata only, never the body**, so no secret
  can leak through it. Denied mutations (`403`) are recorded too. Mounted inside
  the authn layer, given a sink by the composition root. A failed audit write is
  logged at `warn`, never swallowed, and never fails the request (the mutation
  already produced its response).
- `seiva_data_dorm::DormAuditSink` appends to `seiva_audit` (DDL re-exported as
  `audit_migrations`). It opens its **own** session per record rather than joining
  the request transaction, so the transport middleware — which runs above any
  transaction — can use it. A handler that wants field-level detail inside its own
  transaction builds a redacted `AuditEntry` and records it there instead.
- `seiva_testing::RecordingAuditSink` doubles the sink for assertions.

### Proof

Unit tests: the middleware records a mutation's actor/tenant/operation/outcome,
records a denied mutation, and skips reads; redaction masks only sensitive values.
Live (`live_audit_sink_persists_a_redacted_entry`, gated on `SEIVA_DORM_TEST_DSN`)
against real PostgreSQL: a recorded entry is read back with its fields, the
password detail stored as `[redacted]`, and the secret value **never** present in
the row.

## Consequences

**Positive.** Every admin/sensitive mutation (success or denial) is persisted with
who/what/result/correlation; secrets cannot leak through the audit; the base stays
DB-agnostic behind the port; one redaction definition is shared.

**Negative / cost.** The middleware audits *metadata* automatically; a field-level
redacted diff is opt-in per handler (recording a richer entry). The DORM sink
writes on a separate session, so a crash between the mutation's commit and the
audit write can miss a record — acceptable for audit (the outcome, including the
pre-commit state, is still the handler's to record transactionally when required).

## Alternatives considered

- **Audit inside the request transaction (always).** Rejected as the default: the
  transport middleware has no session, and threading a sink into every handler is
  heavier than a metadata middleware. Handlers that need transactional audit still
  can (build and record a redacted entry in their `UnitOfWork`).
- **Reuse `dorm-eventstore`.** Rejected: it is versioned event-sourcing for
  aggregates, not an append-only operational audit; a plain table fits better.
- **Store the full request body.** Rejected outright by the brief — it would
  persist secrets; metadata + opt-in redacted details is the rule.
