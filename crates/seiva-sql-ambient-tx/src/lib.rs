//! The ambient-transaction read-seam shared by the handle-based SQL reference
//! adapters (`seiva-data-sqlx`, `seiva-data-seaorm`; ADR 0030).
//!
//! The "ambient transaction" contract: [`UnitOfWork::transaction`] is scope-based,
//! not handle-based — it passes no transaction handle to the body, so a module's
//! repository reaches the current transaction through a task-local the surrounding
//! `transaction` bound. The two SQL adapters implemented that seam identically; this
//! crate factors out the half that does not depend on the concrete backend:
//!
//! - [`TxState<T>`] — the holder bound into the task-local: the live handle (an
//!   `Option<T>` so it can be taken back out to commit/roll back once the body
//!   returns) plus the [`Scope`] the unit of work opened under.
//! - [`with_current_tx_mut`] / [`with_current_tx_ref`] — borrow the current handle
//!   from an adapter's task-local `key`. Two variants because the borrow is a real
//!   difference, not an accident: sqlx statements need `&mut Transaction`, SeaORM's
//!   take `&DatabaseTransaction`.
//! - [`current_scope`] — read the ambient [`Scope`] without touching the handle.
//! - [`boxed`] — map a concrete error to the port boundary's opaque [`BoxError`].
//!
//! What stays in each adapter: its own `tokio::task_local!` (a `static` cannot be
//! generic, so the key is declared per adapter) and its own `begin`/`commit`/
//! `rollback` (genuinely backend-specific). The DORM gateway does not use this crate
//! at all — its model is closure-scoped (`dorm_core::db::atomic`, ADR 0004), with no
//! handle to bind.
//!
//! [`UnitOfWork::transaction`]: seiva_ports::UnitOfWork::transaction

use seiva_ports::{BoxError, PortResult, Scope};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::LocalKey;

/// Boxes any concrete error as the port boundary's opaque [`BoxError`].
#[must_use]
pub fn boxed<E: std::error::Error + Send + Sync + 'static>(e: E) -> BoxError {
    Box::new(e)
}

/// The error a port returns when driven outside a [`UnitOfWork::transaction`] — the
/// ambient transaction the SQL ports require is simply not bound. `backend` names the
/// adapter (`"sqlx"`, `"SeaORM"`) so the message points at the right tool.
///
/// [`UnitOfWork::transaction`]: seiva_ports::UnitOfWork::transaction
#[must_use]
pub fn no_ambient_tx(backend: &str) -> BoxError {
    format!("no ambient {backend} transaction in scope: call this inside UnitOfWork::transaction")
        .into()
}

/// The state bound into an adapter's task-local for the duration of one unit of work:
/// the live transaction handle `T` (an `Option` so it can be taken back out to
/// commit/roll back once the body returns) and the [`Scope`] it was opened under.
///
/// Built by an adapter's `UnitOfWork::transaction`, bound into its `task_local!`, and
/// read back through [`with_current_tx_mut`] / [`with_current_tx_ref`] /
/// [`current_scope`]. The fields are private: the handle is reached only through the
/// seam functions (so a borrow cannot outlive the lock) and taken out only via
/// [`take`](TxState::take).
pub struct TxState<T> {
    tx: Mutex<Option<T>>,
    scope: Scope,
}

impl<T> TxState<T> {
    /// Wraps a freshly-begun transaction `tx` and its [`Scope`] as the shared state,
    /// ready to bind into a task-local (hence `Arc`).
    #[must_use]
    pub fn new(tx: T, scope: Scope) -> Arc<Self> {
        Arc::new(Self {
            tx: Mutex::new(Some(tx)),
            scope,
        })
    }

    /// Takes the handle back out once the body has returned, to commit or roll back.
    /// Returns `None` if it was already taken (a second call, a bug) — the adapter
    /// maps that to an error rather than panicking.
    pub async fn take(&self) -> Option<T> {
        self.tx.lock().await.take()
    }
}

/// Runs `f` with a **mutable** borrow of the current unit of work's handle, read from
/// the adapter's task-local `key`. The sqlx seam: a statement runs on `&mut **tx`.
///
/// The closure returns a **boxed** future whose lifetime is tied to the borrowed
/// `&mut T` (the `for<'c>` bound), which is what lets the body `.await` while holding
/// the reference. Returns [`no_ambient_tx`] if `key` is unbound (called outside a
/// `UnitOfWork::transaction`).
pub async fn with_current_tx_mut<T, F, R>(
    key: &'static LocalKey<Arc<TxState<T>>>,
    backend: &'static str,
    f: F,
) -> PortResult<R>
where
    T: 'static,
    F: for<'c> FnOnce(&'c mut T) -> Pin<Box<dyn Future<Output = PortResult<R>> + Send + 'c>>,
{
    let state = key
        .try_with(Arc::clone)
        .map_err(|_| no_ambient_tx(backend))?;
    let mut guard = state.tx.lock().await;
    let tx = guard.as_mut().ok_or_else(|| no_ambient_tx(backend))?;
    f(tx).await
}

/// Runs `f` with a **shared** borrow of the current unit of work's handle, read from
/// the adapter's task-local `key`. The SeaORM seam: its connection methods take
/// `&DatabaseTransaction`. Otherwise identical to [`with_current_tx_mut`].
pub async fn with_current_tx_ref<T, F, R>(
    key: &'static LocalKey<Arc<TxState<T>>>,
    backend: &'static str,
    f: F,
) -> PortResult<R>
where
    T: 'static,
    F: for<'c> FnOnce(&'c T) -> Pin<Box<dyn Future<Output = PortResult<R>> + Send + 'c>>,
{
    let state = key
        .try_with(Arc::clone)
        .map_err(|_| no_ambient_tx(backend))?;
    let guard = state.tx.lock().await;
    let tx = guard.as_ref().ok_or_else(|| no_ambient_tx(backend))?;
    f(tx).await
}

/// The [`Scope`] the current unit of work was opened for, or `None` outside a
/// transaction (the `key` is unbound). A tenant-aware repository reads it to
/// constrain its own SQL; the SQL adapters do not auto-filter — the query is the
/// user's.
pub fn current_scope<T: 'static>(key: &'static LocalKey<Arc<TxState<T>>>) -> Option<Scope> {
    key.try_with(|state| state.scope.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A test task-local over a trivial `String` handle, standing in for a real
    // backend transaction — enough to exercise the shared seam end to end.
    tokio::task_local! {
        static CURRENT: Arc<TxState<String>>;
    }

    #[tokio::test]
    async fn seam_is_unbound_outside_a_transaction() {
        assert!(current_scope(&CURRENT).is_none());
        let got = with_current_tx_ref(&CURRENT, "test", |_h| {
            Box::pin(async { Ok::<(), BoxError>(()) })
        })
        .await;
        assert!(got.is_err(), "the seam must error when the key is unbound");
        let msg = got.unwrap_err().to_string();
        assert!(
            msg.contains("no ambient test transaction"),
            "message: {msg}"
        );
    }

    #[tokio::test]
    async fn seam_reads_the_bound_handle_and_scope_then_takes_it_back() {
        let scope = Scope::tenant("acme").expect("a non-empty tenant");
        let state = TxState::new("handle".to_owned(), scope.clone());
        let read = CURRENT
            .scope(Arc::clone(&state), async {
                // The scope is visible to a repository...
                assert_eq!(current_scope(&CURRENT), Some(scope.clone()));
                // ...and so is the handle, mutably.
                with_current_tx_mut(&CURRENT, "test", |h: &mut String| {
                    Box::pin(async move {
                        h.push_str("-used");
                        Ok::<String, BoxError>(h.clone())
                    })
                })
                .await
            })
            .await
            .expect("the bound seam succeeds");
        assert_eq!(read, "handle-used");
        // After the body, the owner can take the handle back out to commit.
        assert_eq!(state.take().await.as_deref(), Some("handle-used"));
        // A second take is empty (what maps to "already consumed" in an adapter).
        assert!(state.take().await.is_none());
    }
}
