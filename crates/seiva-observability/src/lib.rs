//! Delonix Seiva — native observability (ADR 0027).
//!
//! This crate is the **composition-root side** of the framework's observability:
//! it installs the exporters a service *opts into*. The library crates
//! (`seiva-http`, the worker) record against the lightweight [`metrics`] facade
//! unconditionally — a no-op until a recorder is installed. This crate provides
//! that recorder (Prometheus) and a `/metrics` scrape surface, plus an optional
//! OpenTelemetry OTLP tracing layer. Both live behind cargo features, so a build
//! that enables neither links no exporter and pays nothing.
//!
//! - the `metrics` module (feature `metrics`) — install a Prometheus recorder for
//!   the facade and serve `/metrics` on a dedicated bind.
//! - the `tracing` module (feature `tracing`) — install an OTLP tracing layer
//!   configured from the standard `OTEL_*` environment.
//!
//! (Both modules are feature-gated, so they are documented only when their feature
//! is enabled; the links are omitted here to keep the default-feature doc build
//! clean.)
//!
//! # DORM-free
//!
//! This crate links no `dorm-*` crate and no DORM-git-coupled Seiva crate. It is
//! a plain workspace member, built by the default (tokenless) CI.

#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "metrics")]
pub mod metrics;

#[cfg(feature = "tracing")]
pub mod tracing;
