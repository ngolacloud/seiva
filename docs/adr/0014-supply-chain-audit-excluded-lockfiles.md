# ADR 0014 — Supply-chain audit of the excluded, DORM-coupled lockfiles

- Status: Accepted
- Date: 2026-11-05
- Deciders: NgolaCloud
- Follows: ADR 0004 (DORM gateway)

## Context

`cargo-deny` runs in CI over the workspace (default and all-features) — but the
workspace **excludes** the DORM-coupled crates (`seiva-data-dorm`,
`seiva-outbox-relay`, `seiva-rag-worker`, `seiva-mcp`, `apps/dorm-e2e`), so their
dependency trees — DORM's whole graph — are **never audited**. The brief (§9) is
explicit: "A análise default/all-features do workspace principal não cobre
automaticamente tudo"; extend the audit to the excluded lockfiles, record
exceptions with expiry, and never lower the policy to pass.

Auditing those lockfiles immediately surfaced three advisories the workspace
never saw, all in DORM's tree:

- **RUSTSEC-2022-0004** (vulnerability) and **RUSTSEC-2025-0025** (unmaintained):
  `rustc-serialize`, pulled transitively by `eui48` (MAC-address support in
  `postgres-types`). `eui48` uses it only for the MAC Decodable/Encodable codec,
  not the vulnerable untrusted-JSON parser, so the specific overflow is not
  reached here.
- **RUSTSEC-2025-0134** (unmaintained): `rustls-pemfile 2.x`, a thin wrapper
  pulled by `tokio-postgres-rustls` in DORM's TLS path. No known vulnerability.

The fix for all three lives in **DORM's** dependency tree, not Seiva.

## Decision

**Audit each excluded crate's lockfile for RustSec advisories in CI, with a
dedicated `deny.excluded.toml` whose only relaxations are three time-boxed,
reasoned exceptions pointing at DORM as the owner.**

- The token-gated `dorm-gateway` CI job (where DORM resolves) runs
  `cargo deny --config deny.excluded.toml check advisories` per excluded crate,
  after building/linting/testing them. A **new** advisory in DORM's tree now
  fails this gate instead of being invisible.
- `deny.excluded.toml` relaxes **nothing** globally: it is `advisories`-only
  (`yanked = "deny"`), and its `ignore` list names exactly the three advisories
  above, each with a reason, the pulling crate, the owner (DORM) and a review date
  (2026-02-28). The workspace `deny.toml` is untouched — the main gate keeps its
  full force; these crates never enter the workspace graph anyway.

## Consequences

**Positive.** DORM's tree is now audited for advisories in CI; the three known
issues are tracked debt (reasoned, dated, owned) rather than silent gaps; a
regression (a new CVE pulled by DORM) breaks the gate.

**Negative / cost.** Three ignores to re-review at the date, and to drop as DORM
updates (`eui48`/`rustc-serialize`, `rustls-pemfile`). The audit runs only when
`DORM_TOKEN` is configured (like the other DORM jobs); without it the excluded
crates are not built or audited at all — unchanged from before, but worth stating.

## Alternatives considered

- **Add the ignores to the workspace `deny.toml`.** Rejected: it muddies the main
  gate with exceptions for crates the workspace never pulls; a separate
  advisories-only config keeps the main policy pristine and the DORM debt explicit.
- **Run the full `check` (licenses + bans) on the excluded crates.** Deferred:
  §9's priority is the RustSec/CVE gate; DORM's large tree would need its own
  license allow-list curation, a separate follow-up. Advisories first.
- **Ignore the advisories without a config / silently.** Rejected outright by §9
  ("Ausência de advisory não prova segurança", exceptions need expiry).
