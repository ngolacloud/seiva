//! The RAG indexing worker (DORM ADR 0019, brick 3).
//!
//! A model marked `#[derive(Rag)]` is kept indexed in pgvector by draining the
//! `dorm_rag_outbox`: each change embeds what changed and refreshes the model's
//! shadow table. Draining **one batch** is DORM's — [`dorm_rag::worker::run_once`]
//! claims a batch with `FOR UPDATE SKIP LOCKED`, embeds only the chunks whose
//! content hash or embedder changed, upserts them, and returns how many records it
//! handled ([`dorm_rag::worker::WorkerStats`]). This crate is the **loop** the
//! brief assigns to the framework: the surrounding interval, graceful shutdown and
//! metrics come from the Seiva worker runtime ([`delonix_seiva::worker::run_worker`]),
//! so DORM never owns a `loop {}` — that is the whole point of the boundary.
//!
//! DORM ships its own `run` (an infinite poll loop); this crate deliberately does
//! not use it. Running `run_once` under [`run_worker`] gives the same at-least-once
//! indexing with the framework's shutdown and metrics instead of a bare sleep.
//!
//! DORM is a pinned git dependency; like `seiva-outbox-relay` this crate is
//! excluded from the Seiva workspace until the `dorm` feature wires it in.

#![forbid(unsafe_code)]

use std::future::Future;
use std::sync::Arc;

use delonix_seiva::worker::{run_worker, WorkerConfig, WorkerReport};
use dorm_rag::worker::{run_once, WorkerConfig as RagConfig};
use dorm_rag::Embedder;

/// Run DORM's RAG indexer as a **worker**: the framework's loop ([`run_worker`])
/// drives [`run_once`] one pass at a time, with an interval, graceful shutdown and
/// metrics, until `shutdown` resolves.
///
/// The split mirrors [`crate`]'s module docs — one pass drains a batch of the
/// `dorm_rag_outbox` and re-embeds what changed (DORM, the data layer); the loop
/// that repeats it, times it, and stops it cleanly is the framework's. Each pass
/// reports [`dorm_rag::worker::WorkerStats::processed`] as its item count, which
/// the [`WorkerReport`] accumulates and the fast-drain path
/// ([`WorkerConfig::drain_fast_at`]) can key on to burn through a backlog.
///
/// `embedder` is kept generic over [`Embedder`] so the choice of backend (local
/// ONNX, Ollama, an OpenAI-compatible endpoint) stays the caller's. It is shared
/// across passes behind an [`Arc`]; `rag_cfg` is cloned per pass. A DORM error
/// from a pass is bridged to the worker's boundary error type
/// ([`seiva_ports::BoxError`]) — [`run_worker`] logs and counts it but never
/// treats it as fatal, so the worker outlives a transient failure.
///
/// ```no_run
/// use std::time::Duration;
/// use delonix_seiva::worker::WorkerConfig;
/// use dorm_rag::worker::WorkerConfig as RagConfig;
/// use dorm_rag::Embedder;
///
/// async fn start<E: Embedder + Send + Sync + 'static>(embedder: E) {
///     let worker = WorkerConfig::new("rag-index", Duration::from_secs(2)).drain_fast_at(64);
///     let report = seiva_rag_worker::run_rag_worker(
///         worker,
///         std::future::pending::<()>(), // a real caller passes a shutdown signal
///         embedder,
///         RagConfig::default(),
///     )
///     .await;
///     let _ = report.processed;
/// }
/// ```
pub async fn run_rag_worker<E, S>(
    worker: WorkerConfig,
    shutdown: S,
    embedder: E,
    rag_cfg: RagConfig,
) -> WorkerReport
where
    E: Embedder + Send + Sync,
    S: Future<Output = ()>,
{
    let embedder = Arc::new(embedder);
    run_worker(worker, shutdown, move || {
        // Each pass owns its clones, so the future borrows only its own locals.
        let embedder = embedder.clone();
        // RagConfig is Clone — each pass gets its own copy.
        let cfg = rag_cfg.clone();
        async move {
            run_once(&*embedder, &cfg)
                .await
                .map(|stats| stats.processed)
                // Bridge the DORM error to the worker's boundary error type.
                .map_err(|e| Box::new(e) as seiva_ports::BoxError)
        }
    })
    .await
}
