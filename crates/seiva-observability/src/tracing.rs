//! OpenTelemetry OTLP tracing: an opt-in `tracing-opentelemetry` layer that
//! exports spans over OTLP, composed with the framework's existing JSON log
//! stream (12-factor XI — logs stay on stdout; spans go to the collector).
//!
//! [`init`] installs the layer and a **W3C trace-context propagator**, so a
//! `traceparent`/`tracestate` extracted at the HTTP boundary (and, in a later
//! slice, carried through the CloudEvents envelope) continues one logical trace
//! across services after a `seiva module split`. The exporter endpoint, protocol
//! and sampling are read from the standard `OTEL_*` environment
//! (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_TRACES_SAMPLER*`, …).
//!
//! Without the `tracing` feature this module is absent and the existing JSON
//! subscriber (`seiva_server::observability::init`) is unchanged — zero cost.

use std::fmt;

use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// The `service.name` used when neither [`TracingConfig::service_name`] nor
/// `OTEL_SERVICE_NAME` is set — a span attribute, not a metric label.
pub const DEFAULT_SERVICE_NAME: &str = "seiva-service";

/// The instrumentation scope name the framework's own tracer reports.
const INSTRUMENTATION_SCOPE: &str = "seiva-observability";

/// Opt-in tracing configuration. Everything transport-related (endpoint,
/// protocol, timeout, sampling) is read by the OTLP exporter and the SDK from the
/// standard `OTEL_*` environment; only the service name is surfaced here so a
/// composition root can set it in code.
#[derive(Debug, Clone, Default)]
pub struct TracingConfig {
    /// The `service.name` resource attribute. Falls back to `OTEL_SERVICE_NAME`,
    /// then [`DEFAULT_SERVICE_NAME`].
    pub service_name: Option<String>,
}

/// An error raised while installing the OTLP tracing pipeline.
#[derive(Debug)]
#[non_exhaustive]
pub enum TracingError {
    /// The OTLP span exporter could not be built (e.g. an invalid endpoint or a
    /// transport configuration error).
    Exporter(String),
}

impl fmt::Display for TracingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TracingError::Exporter(m) => write!(f, "failed to build the OTLP span exporter: {m}"),
        }
    }
}

impl std::error::Error for TracingError {}

/// Installs the OTLP tracing pipeline and the global W3C trace-context propagator,
/// then composes a `tracing-opentelemetry` span layer with the JSON log stream and
/// the `RUST_LOG` filter.
///
/// Returns the [`SdkTracerProvider`] so the composition root can flush and shut it
/// down on exit (a batch exporter buffers spans; dropping without
/// [`SdkTracerProvider::shutdown`] can lose the tail). The subscriber is installed
/// with `try_init`, so a second call (or a process that already set a subscriber)
/// is a no-op rather than a panic.
///
/// # Errors
///
/// Returns [`TracingError::Exporter`] if the OTLP exporter cannot be built.
pub fn init(config: TracingConfig) -> Result<SdkTracerProvider, TracingError> {
    // The exporter reads OTEL_EXPORTER_OTLP_ENDPOINT / protocol / timeout from the
    // environment; gRPC over tonic is the configured transport.
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .build()
        .map_err(|e| TracingError::Exporter(e.to_string()))?;

    let service_name = config
        .service_name
        .or_else(|| std::env::var("OTEL_SERVICE_NAME").ok())
        .unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_string());

    let resource = Resource::builder().with_service_name(service_name).build();

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();

    // W3C `traceparent`/`tracestate` in and out: the single propagator the HTTP
    // boundary (and the event envelope, next slice) use to continue a trace.
    global::set_text_map_propagator(TraceContextPropagator::new());
    global::set_tracer_provider(provider.clone());

    let tracer = provider.tracer(INSTRUMENTATION_SCOPE);
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // Spans export to OTLP; the JSON event stream stays on stdout (12-factor XI).
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_current_span(false),
        )
        .with(otel_layer)
        .try_init();

    Ok(provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_service_name_is_used_when_unset() {
        let cfg = TracingConfig::default();
        assert!(cfg.service_name.is_none());
        // The constant is the documented fallback.
        assert_eq!(DEFAULT_SERVICE_NAME, "seiva-service");
    }
}
