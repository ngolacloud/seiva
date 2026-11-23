# ADR 0039 — `seiva-scim`: SCIM 2.0 user and group provisioning

- Status: Proposed
- Date: 2026-11-23
- Deciders: NgolaCloud
- Follows: ADR 0006 (identity and authentication), ADR 0009 (per-module admin registration)

## Context

An enterprise customer does not create users in your platform by hand. Their
identity provider — Okta, Entra ID, Google Workspace — is the source of truth,
and it expects to *push* the directory: provision a user when an employee joins,
patch attributes when they change teams, and **de-provision** the moment they
leave. The protocol for that push is **SCIM 2.0** (System for Cross-domain
Identity Management, RFC 7643/7644). A platform without a SCIM endpoint cannot be
sold into an organisation that runs centralised joiner/mover/leaver automation —
it falls back to manual account admin, which is the thing the customer is paying
to avoid.

delonix-meet already carries this: a SCIM surface mapping an IdP's directory onto
its orgs/branches/seats. It is pure mechanism welded to the product's directory.
Every platform that onboards enterprises needs the same `/Users` and `/Groups`
CRUD, the same filtering and pagination, the same idempotent sync — while *what a
user maps to* (a seat, a branch, a role) is domain policy. Classic upstreaming
split.

The forces:

- **The IdP is the writer; we are the sink.** Provisioning is an inbound,
  server-authoritative flow: the IdP `POST`s/`PATCH`es, we reconcile. Our identity
  store (ADR 0006) is the target; the SCIM layer is an adapter in front of it, not
  a second directory.
- **Idempotent by `externalId`.** An IdP retries. The same user pushed twice must
  not create two accounts. `externalId` (the IdP's stable key) is the idempotency
  anchor: a create for a known `externalId` is a no-op-or-update, never a
  duplicate. The whole sync must be **reconcilable** — re-running it converges.
- **De-provisioning is a first-class operation, not a `DELETE`.** A leaver is
  deactivated/soft-deleted (`active: false`), preserving audit history and
  referential integrity, not hard-deleted out from under the data that references
  them. The IdP's `active` flag drives account state.
- **Tenant-scoped and fail-closed.** A SCIM token belongs to exactly one tenant;
  every `/Users` and `/Groups` operation is scoped to that tenant with no
  fail-open path (ADR 0019). An IdP for tenant A can never read, patch or
  de-provision tenant B's directory.
- **PII minimisation.** The directory carries names, emails, employee numbers.
  Only the attributes the product maps are stored; unmapped SCIM attributes are
  not silently hoarded, and secrets/tokens never land in logs.

## Decision

Add a new workspace member **`seiva-scim`** — a SCIM 2.0 provisioning surface,
data-layer-free, feature-gated, mapping onto the identity/directory of ADR 0006.

- **`/Users` and `/Groups`** — the SCIM 2.0 resource endpoints: `POST` (create),
  `GET` (read/by-id), `PUT` (replace), **`PATCH`** (RFC 7644 partial update, the
  operation IdPs lean on), and `DELETE` (routed to deactivate, not hard-delete).
  Responses carry the SCIM `meta`/`schemas` envelope and the `id`↔`externalId`
  pairing.
- **Filtering** — the SCIM `filter` query (e.g. `userName eq "a@b.com"`,
  `externalId eq "..."`), parsed into a **bounded, allowlisted** attribute set and
  pushed to the store as an indexed lookup — never an arbitrary expression against
  the database.
- **Pagination** — `startIndex`/`count` cursoring with a server-capped page size,
  so an IdP enumerating a large directory cannot request an unbounded scan.
- **Bulk** — the SCIM `/Bulk` endpoint for batched operations, each entry carrying
  its own idempotency so a partial failure is per-entry, not all-or-nothing.
- **Idempotent sync by `externalId`** — a `DirectorySink` port maps a SCIM
  resource to the product's user/group via `externalId`: an existing mapping
  updates, an absent one creates, and a re-push converges. The port is the seam
  where the product declares **org↔directory mapping** and any **custom
  attributes** (a seat, a branch, a role claim).
- **De-provisioning** — `active: false` (via `PATCH` or `DELETE`) deactivates the
  account through the identity store, preserving the row and its audit trail.
- **Auth and scope** — a bearer token issued to the IdP (via ADR 0038 or a
  provisioning key) authenticates every request; the token's tenant scopes every
  operation, fail-closed.

`tokio` enters with `default-features = false`; the `DirectorySink` is a port with
an in-memory double for tests and a DORM-backed adapter behind the `dorm` feature.
The provisioned resources register as `AdminResource`s (ADR 0009) so an operator
sees the synced directory in the console.

## Alternatives considered

- **A bespoke provisioning API per product.** Rejected: IdPs speak SCIM, not your
  REST dialect. A non-SCIM endpoint means every enterprise customer writes a
  custom connector — a sales blocker and a maintenance tax. SCIM is the interop
  contract the buyers already run.
- **Hard-delete on `DELETE`.** Rejected: a leaver's user row is referenced by
  meetings, audit entries and ownership records. Deactivation preserves integrity
  and history; a hard delete orphans data and erases the audit trail — the
  opposite of what a compliance-driven enterprise wants.
- **Trust the IdP's `id` as the key.** Rejected: our `id` and the IdP's are
  different namespaces. `externalId` is the IdP's stable key *by design*; anchoring
  idempotency on it is what the spec intends and what makes re-sync converge.
- **Fold SCIM into `seiva-identity`.** Rejected: `seiva-identity` is a
  transport-free verifier (ADR 0006); SCIM is an inbound HTTP provisioning surface
  with its own schema envelope, filter grammar and bulk semantics. A separate
  crate keeps the verifier network-free and lets provisioning evolve independently.

## Consequences

- **Performance.** Filters compile to **indexed** attribute lookups over a bounded
  allowlist (no arbitrary query); pagination is server-capped; `/Bulk` batches an
  IdP's initial import. The crate links no runtime and is zero-cost for a platform
  that does not offer provisioning.
- **Security.** Every request is authenticated by the IdP's bearer token and
  **tenant-scoped fail-closed** (ADR 0019) — cross-tenant provisioning is
  impossible; only mapped attributes are stored (**PII minimisation**); secrets
  never log. An IdP for one tenant cannot touch another's directory.
- **Resilience.** `externalId` idempotency means a retried or replayed push never
  duplicates an account; the whole sync is **reconcilable** — re-running it
  converges to the IdP's state; de-provisioning is a reversible deactivation that
  preserves audit history and referential integrity.
- delonix-meet stops maintaining a hand-built SCIM surface and keeps only its
  **org↔directory mapping** (which SCIM attribute becomes a seat/branch/role) as
  policy in the `DirectorySink`; every future Seiva platform gets enterprise
  joiner/mover/leaver automation for free.
- It does **not** make Seiva an identity provider: SCIM is the *inbound* sink that
  lets an external IdP own the directory, complementing the OIDC login flow
  (ADR 0040) on the authentication side.
