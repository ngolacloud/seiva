# ADR 0043 — An append-only, hash-chained audit log: a tamper-evident `AuditSink` implementation

- Status: Proposed
- Date: 2026-11-23
- Deciders: NgolaCloud
- Follows: ADR 0002 (core ports and in-memory doubles), ADR 0013 (persistent audit of administrative mutations), ADR 0019 (fail-closed authorization)

## Context

Seiva already has the **`AuditSink` port** and the ADR 0013 middleware that records
every administrative mutation (actor, tenant, operation, outcome, correlation id,
redacted details) through it, with a DORM-backed sink appending to `seiva_audit`.
That answers *"was it recorded?"*. It does not yet answer *"can the record be
trusted — has anyone edited or deleted a row after the fact?"*. For a sovereign
platform, an operator (or an attacker who reaches the database) could `UPDATE` or
`DELETE` an audit row and erase their tracks, and nothing would detect it. A
regulated or security-sensitive deployment needs an audit log that is
**append-only and tamper-evident**: writes only, and any after-the-fact edit is
*visible*.

This ADR does not replace ADR 0013; it **extends** its `AuditSink` with an
implementation whose storage is immutable and whose entries are chained so that
tampering is detectable, plus retention and export.

The forces:

- **Append-only, enforced, not merely intended.** "Don't edit audit rows" as a
  convention is worthless against a compromised operator. The implementation must
  make `UPDATE`/`DELETE` *impossible or detectable*: a write-only sink, database
  grants that allow `INSERT` only, and revoked mutate/delete on the audit table —
  the storage itself refuses the edit.
- **Tamper-evidence is a cryptographic property, not a log level.** Append-only
  storage stops casual edits, but a privileged actor who can rewrite the table
  still needs to be *caught*. A **hash chain** — each entry stores the hash of the
  previous entry plus its own content — means altering or removing any row breaks
  the chain from that point on, and a verification pass finds exactly where.
- **An audit that can be silently skipped is not an audit.** For the operations
  that legally *must* be audited, a failed audit write must **fail the operation**
  (fail-closed, ADR 0019), not be logged-and-swallowed. This is a stricter posture
  than ADR 0013's best-effort middleware, applied to the critical subset.
- **PII minimisation.** Audit entries describe *who did what to what*, not the
  secret payload (ADR 0013 already redacts). Before/after state is stored as a
  **hash** by default, not the raw values, so the log proves a change occurred
  without itself becoming a second copy of sensitive data.
- **Retention is policy; mechanism is partitioning.** How long to keep, and what
  the law requires, are the product's/operator's policy. The framework provides
  time-partitioning, retention enforcement, and export so that policy is cheap to
  apply.

## Decision

Add an **append-only, hash-chained `AuditSink` implementation** — an additional
adapter behind the existing port, not a new port — plus its verification and
export tooling.

- **The chained entry** — extends `AuditEntry` (ADR 0013: actor, action, resource,
  scope, timestamp, request-id, redacted details) with a `prev_hash` and an
  `entry_hash = H(prev_hash ‖ canonical(entry))`. `before`/`after` state is stored
  as a hash by default (the raw diff only when the product opts in and it is
  non-sensitive). Entries form a per-tenant (or global) chain.
- **Write-only storage.** The DORM/sqlx adapter persists to an audit table whose
  grants permit **`INSERT` only** — `UPDATE` and `DELETE` are revoked for the
  application role; combined with time-partitioning, old partitions can be sealed.
  The sink exposes **no** update or delete method at the port level either.
- **A verification pass** — `verify_chain(range)` recomputes each `entry_hash` from
  `prev_hash` + content and reports the first break, so tampering (an edited or
  removed row) is located, not merely suspected. It can run on a schedule
  (`seiva-jobs`, ADR 0034) and raise an alert on a break.
- **Fail-closed for the critical subset.** A `CriticalAudit` wrapper writes the
  entry **durably before** the audited operation commits and makes a failed write
  **fail the operation**. ADR 0013's metadata middleware stays best-effort for the
  broad mutation stream; this stricter mode is opt-in for the legally-required
  actions.
- **Retention and export.** Time-partitioning (monthly, say) makes retention a
  partition drop and keeps the hot path small; an `export(range)` produces a signed
  archive for cold storage or an external SIEM, carrying the chain so the export is
  independently verifiable.

The policy — *which* actions demand critical fail-closed audit, and the legal
retention window — stays with the product (ADR 0013 already catalogues the admin
mutations). The mechanism — immutability, chaining, verification, export — is the
framework's.

## Alternatives considered

- **Keep only ADR 0013's best-effort table.** Rejected for regulated deployments:
  a mutable table with a best-effort writer proves nothing against an operator who
  edits it; append-only + hash-chain is what makes the log *trustworthy*.
- **Ship audit to an external SIEM and call it immutable.** Rejected as the base:
  that is a valid *export target*, but the first-party write must be tamper-evident
  on its own so the pipeline to the SIEM is itself auditable. Export complements,
  it does not replace.
- **Sign every entry individually instead of chaining.** Rejected: per-entry
  signatures prove authenticity but not *completeness* — a deleted row leaves no
  gap. A hash chain detects deletion and reordering, which is the threat.
- **Store the full before/after values always.** Rejected: that re-copies
  sensitive data into the audit log (the thing ADR 0013 forbids). Hash-by-default,
  raw-on-explicit-opt-in keeps the log minimal.

## Consequences

- **Performance.** Writes are **sequential appends**; time-partitioning keeps the
  live partition small and indexes tight; an optional async write path (via
  `seiva-jobs`) absorbs bursts **without losing order**, since the chain enforces
  ordering independently of write timing.
- **Security.** The log is **immutable** (INSERT-only grants, no update/delete at
  the port); the **hash chain** makes any edit, deletion or reordering detectable;
  before/after is hashed to minimise PII; the critical subset is **fail-closed**
  (ADR 0019) — a failed audit write aborts the audited operation rather than
  proceeding unrecorded.
- **Resilience.** The chain is **independently verifiable** at any time, so trust
  does not depend on the writer's good behaviour; the critical path writes durably
  **before** the operation commits, so a crash cannot leave a committed operation
  unaudited; export carries the chain for independent off-site verification.
- delonix-meet's regulated-audit requirement is met by swapping in this sink behind
  the port it already uses — no call-site changes — and every platform that must
  prove its audit was not tampered with gains it for free.
