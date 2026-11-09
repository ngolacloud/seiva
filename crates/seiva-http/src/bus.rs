//! The two pipeline behaviors that bind to the **request context** (ADR 0020).
//!
//! [`seiva_cqrs`] ships the transport-free behaviors — [`Validate`] (keyed off the
//! message's own [`Validated`] contract) and [`Transactional`] (keyed off an
//! injected [`UnitOfWork`] port). The remaining two behaviors of ADR 0020 need
//! state the *transport* supplies per request, so they live here, in the HTTP
//! crate, next to the [`Ctx`] they are built from:
//!
//! - [`Authorize`] — the **pre-transaction**, fail-closed authorization check
//!   (ADR 0019). It holds the caller's [`Principal`] and the operation's
//!   [`RequiredPermission`] (the route policy from the facade descriptor) and
//!   rejects a caller that lacks the permission *before* delegating — so no
//!   transaction opens and the inner handler never observes an unauthorized call.
//! - [`Idempotent`] — the **intra-transaction** durable-idempotency check
//!   (ADR 0011). It holds an [`IdempotencyStore`] and the request's `scoped_key`
//!   + `fingerprint`, claims the key, and runs the handler at most once per key.
//!
//! # These are the same shape as every other behavior
//!
//! A behavior is a handler that wraps an inner handler (exactly like
//! [`Instrument`](seiva_cqrs::Instrument) / [`CacheQueries`](seiva_cqrs::CacheQueries)),
//! so the four compose by **nesting** and stay fully typed and monomorphised — no
//! runtime registry, no `dyn`. The write order ADR 0020 fixes is:
//!
//! ```text
//! Authorize → Validate → Transactional{ Idempotent → handler }
//! ```
//!
//! and in Rust that is written inside-out around the handler:
//!
//! ```text
//! Mediator::new(
//!     Authorize::new(
//!         Validate::new(
//!             Transactional::new(
//!                 Idempotent::new(handler, store, scoped_key, fingerprint, ttl),
//!                 uow,
//!             ),
//!         ),
//!         principal, required,
//!     ),
//! )
//! ```
//!
//! # Why the nesting order is a correctness invariant, not a preference
//!
//! [`Authorize`] and [`Validate`] sit **outside** [`Transactional`] on purpose: an
//! unauthorized or malformed request must be rejected *before* a transaction opens,
//! both to fail fast and so a denied call leaves no transactional trace.
//! [`Idempotent`] sits **inside** [`Transactional`] for the opposite reason: the
//! key reservation ([`begin`](IdempotencyStore::begin)/[`finish`](IdempotencyStore::finish)),
//! the handler's mutation and the outbox write must commit as **one** atom, so a
//! committed key always has its effect and a rolled-back one has none (ADR 0011
//! rejected an *outer* idempotency middleware precisely because it runs above any
//! transaction and cannot stay atomic). Reordering these breaks the guarantee; the
//! integration test `tests/bus_pipeline.rs` pins the order by observation.
//!
//! # How each behavior gets its context
//!
//! Both are built **per request** from a [`Ctx`] (and, for `Idempotent`,
//! the request body): the [`Principal`] and the operation path come from the `Ctx`;
//! the required permission comes from the facade method's descriptor; the
//! idempotency `scoped_key`/`fingerprint` are folded from the `Ctx` identity +
//! coordinates + `Idempotency-Key` + body by [`scoped_key`] /
//! [`fingerprint`] — the very helpers the HTTP idempotency path
//! already uses, reused here rather than re-derived. The authorization rule itself
//! is the shared [`require_permission`], the same
//! fail-closed check the tower [`permission_guard`](crate::permission_guard) runs,
//! so the guard and the behavior can never diverge.
//!
//! # Scope (Phase 2)
//!
//! This module proves the **composition** of the behaviors around the `Ctx`; it is
//! driven by the integration test with doubles, not by a live HTTP server. Routing
//! a real module (orders) through the bus instead of calling its service directly
//! is **Phase 3** — nothing here forces an existing controller through the pipeline
//! (opt-in is the adoption mechanism, ADR 0020).
//!
//! [`Validate`]: seiva_cqrs::Validate
//! [`Validated`]: seiva_cqrs::Validated
//! [`Transactional`]: seiva_cqrs::Transactional
//! [`UnitOfWork`]: seiva_ports::UnitOfWork

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;

use seiva_cqrs::{Command, CommandHandler, Query, QueryHandler};
use seiva_identity::Principal;
use seiva_ports::{BoxError, IdempotencyOutcome, IdempotencyStore, PortResult};

use crate::ctx::Ctx;
use crate::guard::require_permission;
use crate::idempotency::{fingerprint, in_progress_problem, mismatch_problem, scoped_key};
use crate::problem::Problem;
use crate::RequiredPermission;

/// The default reservation lifetime for an idempotency key when a caller does not
/// choose one (24 hours). Long enough to absorb a client's retries and a transient
/// outage, short enough that the store does not keep completed responses forever.
pub const DEFAULT_IDEMPOTENCY_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// A **pre-transaction**, fail-closed authorization behavior (ADR 0019).
///
/// It wraps an inner handler and holds the request's [`Principal`] and the
/// operation's [`RequiredPermission`] (the facade method's `requires` policy). On
/// dispatch it runs the shared [`require_permission`]
/// check and **only** delegates when it passes; on denial it returns a `403`
/// [`Problem`] boxed as the port error and **never calls the inner handler** — so a
/// caller without the permission cannot reach the mutation and, because this
/// behavior is composed *outside* [`Transactional`](seiva_cqrs::Transactional), no
/// transaction is ever opened for the denied call.
///
/// It implements both [`CommandHandler`] and [`QueryHandler`], so the same guard
/// serves a write (`Authorize → Validate → Transactional{…}`) and a read
/// (`Authorize → CacheQueries → handler`). An empty requirement (an unguarded
/// operation) always passes, matching the tower [`permission_guard`](crate::permission_guard).
///
/// The returned `403` is **status-bearing**: it travels inside the port's
/// `BoxError` as a [`Problem`], and [`Problem::from`] recovers it verbatim at the
/// edge (it is not remapped to `500`).
pub struct Authorize<H> {
    inner: H,
    principal: Principal,
    required: RequiredPermission,
    /// The operation path (from the `Ctx`), used only to set the `403` problem's
    /// `instance`; empty when the behavior was built without a request.
    instance: String,
}

impl<H> Authorize<H> {
    /// Wraps `inner`, checking `principal` against `required` before each dispatch.
    ///
    /// Use [`from_ctx`](Self::from_ctx) to build directly from a request
    /// [`Ctx`]; this lower-level constructor is for a caller that
    /// already holds the principal and policy (and leaves the problem `instance`
    /// unset).
    pub fn new(inner: H, principal: Principal, required: RequiredPermission) -> Self {
        Self {
            inner,
            principal,
            required,
            instance: String::new(),
        }
    }

    /// Builds the behavior from a request [`Ctx`] and the operation's
    /// [`RequiredPermission`] (read from the facade descriptor): the caller's
    /// [`Principal`] and the path for the `403` `instance` are taken from the `Ctx`.
    /// This is the idiomatic per-request constructor.
    pub fn from_ctx(inner: H, ctx: &Ctx, required: RequiredPermission) -> Self {
        Self {
            inner,
            principal: ctx.principal().clone(),
            required,
            instance: ctx.path().to_owned(),
        }
    }

    /// Borrows the wrapped inner handler/behavior.
    pub fn inner(&self) -> &H {
        &self.inner
    }

    /// Runs the fail-closed check, enriching the denial problem with the operation
    /// path and boxing it as the port error so its status survives to the edge.
    fn authorize(&self) -> PortResult<()> {
        require_permission(&self.principal, self.required.0).map_err(|problem| {
            let problem = if self.instance.is_empty() {
                problem
            } else {
                problem.with_instance(self.instance.clone())
            };
            BoxError::from(problem)
        })
    }
}

impl<C, H> CommandHandler<C> for Authorize<H>
where
    C: Command,
    H: CommandHandler<C>,
{
    async fn handle(&self, cmd: C) -> PortResult<C::Output> {
        // Fail-closed: a denied caller never reaches the handler, and because this
        // behavior sits outside `Transactional`, no transaction opens for it.
        self.authorize()?;
        self.inner.handle(cmd).await
    }
}

impl<Q, H> QueryHandler<Q> for Authorize<H>
where
    Q: Query,
    H: QueryHandler<Q>,
{
    async fn handle(&self, query: Q) -> PortResult<Q::Output> {
        // Same guard on the read side (`Authorize → CacheQueries → handler`).
        self.authorize()?;
        self.inner.handle(query).await
    }
}

/// An **intra-transaction**, durable-idempotency behavior (ADR 0011).
///
/// It wraps an inner handler and holds an [`IdempotencyStore`] `S`, the request's
/// `scoped_key` and `fingerprint`, and the reservation `ttl`. On dispatch it claims
/// the key through [`begin`](IdempotencyStore::begin) *before* delegating and then:
///
/// - [`Fresh`](IdempotencyOutcome::Fresh) — runs the inner handler, serialises its
///   output to JSON, persists it with [`finish`](IdempotencyStore::finish), and
///   returns the output. The handler runs **exactly once** for this key.
/// - [`Replay`](IdempotencyOutcome::Replay) — deserialises and returns the stored
///   response **without running the handler**, so a retried request produces one
///   effect, not two.
/// - [`InProgress`](IdempotencyOutcome::InProgress) /
///   [`Mismatch`](IdempotencyOutcome::Mismatch) — returns a `409` [`Problem`]
///   (the key is still in flight, or was reused for different content) and does
///   **not** execute. The `409` is status-bearing: it rides inside the port's
///   `BoxError` and [`Problem::from`] recovers it verbatim (it is not remapped).
///
/// # This behavior must be nested *inside* `Transactional`
///
/// `begin`/handler/`finish` and the outbox write have to commit as one atom
/// (ADR 0011). Compose it as the innermost link of the transactional scope —
/// `Transactional::new(Idempotent::new(handler, …), uow)` — so the key reservation
/// commits with the mutation it guards. Placed *outside* a transaction it would
/// reserve a key that a later rollback could not release, which is exactly the
/// failure mode ADR 0011 avoided by rejecting an outer idempotency middleware.
///
/// Only [`CommandHandler`] is implemented, by design: idempotency guards a
/// *mutation*; a side-effect-free [`Query`] is neither reserved nor replayed.
///
/// # Bounds, and why
///
/// - `C::Output: Serialize` — the handler's result is stored as JSON for replay.
/// - `C::Output: DeserializeOwned` — a replayed response is decoded from that JSON.
///
/// The response is carried as opaque bytes by the store ([`IdempotencyOutcome`]
/// moves `Vec<u8>`); this behavior owns the (JSON) encoding, matching the HTTP
/// idempotency path's canonicalisation.
pub struct Idempotent<H, S> {
    inner: H,
    store: S,
    scoped_key: String,
    fingerprint: String,
    ttl: Duration,
}

impl<H, S> Idempotent<H, S> {
    /// Wraps `inner`, deduplicating on a precomputed `scoped_key`/`fingerprint`.
    ///
    /// Build the two values with [`scoped_key`] /
    /// [`fingerprint`], or use [`from_ctx`](Self::from_ctx) to
    /// derive them from a request [`Ctx`] and body.
    pub fn new(inner: H, store: S, scoped_key: String, fingerprint: String, ttl: Duration) -> Self {
        Self {
            inner,
            store,
            scoped_key,
            fingerprint,
            ttl,
        }
    }

    /// Builds the behavior from a request [`Ctx`] and the request
    /// `body`, folding the `scoped_key` (caller tenant + subject + method + path +
    /// `Idempotency-Key`) and the `fingerprint` (method + path + canonical JSON
    /// body) with the shared [`scoped_key`] /
    /// [`fingerprint`] helpers.
    ///
    /// Returns [`None`] when the `Ctx` carries **no** `Idempotency-Key`: idempotency
    /// is opt-in per request (ADR 0011), so a caller that did not ask for it is
    /// dispatched without this behavior rather than silently deduplicated. `query`
    /// is `None` because the [`Ctx`] does not retain the query string;
    /// a caller that fingerprints the query uses [`new`](Self::new) directly.
    pub fn from_ctx(
        inner: H,
        store: S,
        ctx: &Ctx,
        body: &serde_json::Value,
        ttl: Duration,
    ) -> Option<Self> {
        let key = ctx.idempotency_key()?;
        let scoped = scoped_key(ctx.principal(), ctx.method(), ctx.path(), key);
        let fp = fingerprint(ctx.method(), ctx.path(), None, body);
        Some(Self::new(inner, store, scoped, fp, ttl))
    }

    /// Borrows the wrapped inner handler/behavior.
    pub fn inner(&self) -> &H {
        &self.inner
    }

    /// The store-facing scoped key this behavior claims.
    #[must_use]
    pub fn scoped_key(&self) -> &str {
        &self.scoped_key
    }

    /// The request fingerprint pinned to this key.
    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

impl<C, H, S> CommandHandler<C> for Idempotent<H, S>
where
    C: Command,
    C::Output: Serialize + DeserializeOwned,
    H: CommandHandler<C>,
    S: IdempotencyStore,
{
    async fn handle(&self, cmd: C) -> PortResult<C::Output> {
        match self
            .store
            .begin(&self.scoped_key, &self.fingerprint, self.ttl)
            .await?
        {
            IdempotencyOutcome::Fresh => {
                // First claim: run the mutation once, then persist its response so a
                // later claim replays it. All of this runs inside the surrounding
                // `Transactional` scope, so the reservation commits with the effect.
                let output = self.inner.handle(cmd).await?;
                let bytes = serde_json::to_vec(&output)?;
                self.store.finish(&self.scoped_key, &bytes).await?;
                Ok(output)
            }
            IdempotencyOutcome::Replay(bytes) => {
                // Already completed: return the stored response, run nothing.
                let output = serde_json::from_slice(&bytes)?;
                Ok(output)
            }
            // Status-bearing 409s: boxed as `Problem`, recovered verbatim at the edge.
            IdempotencyOutcome::InProgress => Err(BoxError::from(in_progress_problem())),
            IdempotencyOutcome::Mismatch => Err(BoxError::from(mismatch_problem())),
        }
    }
}

// `Problem` is `Send + Sync + 'static` and implements `std::error::Error`, so the
// blanket `From<E> for Box<dyn Error + Send + Sync>` boxes it into the port's
// `BoxError`; the helpers above make that explicit for the reader.
const _: fn() = || {
    fn _assert_boxable<E: std::error::Error + Send + Sync + 'static>() {}
    _assert_boxable::<Problem>();
};
