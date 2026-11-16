//! `seiva-resilience` — the framework's production resilience primitives.
//!
//! A Seiva service sits between its callers and its dependencies (a database, a
//! broker, another service's HTTP API). Any of those can slow down, time out or
//! fall over. This crate carries the three guardrails that keep one failing
//! dependency from taking the whole service with it:
//!
//! - [`CircuitBreaker`] — stops hammering a dependency that is already down
//!   (closed → open → half-open → closed), shedding load and probing recovery.
//! - [`RetryPolicy`] — retries a transient failure with exponential backoff, on
//!   the **same schedule** as the transactional-outbox relay
//!   (`seiva_events::RelayPolicy`), so the framework has one retry cadence.
//! - [`timeout()`] — cuts an operation off with a typed [`TimedOut`] error if it
//!   outlives its budget.
//!
//! # Composing them
//!
//! The three are orthogonal and compose from the inside out — timeout each
//! attempt, retry the transient failures, and guard the whole thing with a
//! breaker so a dependency that stays down stops being called at all:
//!
//! ```no_run
//! # use std::time::Duration;
//! # use std::sync::Arc;
//! # use seiva_resilience::{CircuitBreaker, BreakerConfig, RetryPolicy, timeout};
//! # #[derive(Debug)] struct MyError;
//! # impl std::fmt::Display for MyError {
//! #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("err") }
//! # }
//! # impl std::error::Error for MyError {}
//! # async fn call_dependency() -> Result<u32, MyError> { Ok(1) }
//! # async fn demo(breaker: CircuitBreaker, retry: RetryPolicy) {
//! let outcome = breaker
//!     .call(async {
//!         retry
//!             .retry(|| async {
//!                 // Each attempt gets its own timeout budget.
//!                 timeout(Duration::from_secs(2), call_dependency())
//!                     .await
//!                     .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
//!                     .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
//!             })
//!             .await
//!     })
//!     .await;
//! # let _ = outcome;
//! # }
//! ```
//!
//! # Data-layer-free
//!
//! This crate composes futures and times transitions off a monotonic clock. It
//! never touches a database, a broker or the DORM gateway, so it lives in the
//! default workspace members and carries no `dorm` feature.

#![forbid(unsafe_code)]

mod breaker;
mod retry;
pub mod time;
mod timeout;

pub use breaker::{BreakerConfig, BreakerError, BreakerState, CircuitBreaker};
pub use retry::RetryPolicy;
pub use timeout::{timeout, TimedOut};
