//! The event-boundary half of W3C trace-context propagation (ADR 0027): inject the
//! current trace context into an outgoing event and extract an incoming one onto a
//! handling span, so a choreography reads as one distributed trace across the
//! `outbox → broker → subscriber` hop after a `seiva module split`.
//!
//! This is the **facade** the data-layer-free library crates call: `seiva-messaging`
//! carries the `traceparent`/`tracestate` as plain envelope attributes and knows
//! nothing of OpenTelemetry; `seiva-events` (and the DORM-backed drain) call
//! [`inject_current`] when publishing and [`set_parent`] when handling. The heavy
//! runtime lives behind the crate's `tracing` feature and reuses the **single**
//! global W3C propagator installed by `crate::tracing::init` (a code-span, not an
//! intra-doc link: that item is behind the `tracing` feature and absent from the
//! default-feature doc build) — this module never installs a second one.
//!
//! **Zero cost when unused.** Without the `tracing` feature both functions are
//! no-ops: [`inject_current`] returns an empty [`TraceContext`] and [`set_parent`]
//! does nothing, so a build that links no exporter pays nothing and stays
//! DORM-free. The same calls light up — with no change at the call site — once a
//! service opts into tracing at its composition root.

/// A W3C trace-context pair as it travels on a message envelope: the `traceparent`
/// (the parent span the consumer links to) and the optional vendor `tracestate`.
///
/// It is the transport DTO between the CloudEvents envelope (plain strings, no
/// OpenTelemetry) and the propagator in this facade — which is what keeps
/// `seiva-messaging` free of any tracing runtime.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraceContext {
    /// The W3C `traceparent` (`00-<trace-id>-<span-id>-<flags>`), or `None` when no
    /// context was active at publish time.
    pub traceparent: Option<String>,
    /// The W3C `tracestate` vendor extension, carried verbatim; only meaningful
    /// when `traceparent` is set.
    pub tracestate: Option<String>,
}

impl TraceContext {
    /// `true` when there is no context to propagate (no `traceparent`).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.traceparent.is_none()
    }
}

/// Capture the current trace context as a [`TraceContext`] for an outgoing event —
/// the publisher calls this when writing to the outbox so the emitting span travels
/// on the envelope.
///
/// With the `tracing` feature it reads the current span's OpenTelemetry context and
/// injects it through the globally-installed W3C propagator; without the feature (or
/// with no propagator installed) it returns an empty context — a no-op.
#[must_use]
pub fn inject_current() -> TraceContext {
    #[cfg(feature = "tracing")]
    {
        use opentelemetry::global;
        use std::collections::HashMap;
        use tracing_opentelemetry::OpenTelemetrySpanExt;

        // The OpenTelemetry context the current `tracing` span is bridged to; empty
        // when nothing installed the OTel layer, so the propagator injects nothing.
        let cx = tracing::Span::current().context();
        let mut carrier: HashMap<String, String> = HashMap::new();
        global::get_text_map_propagator(|propagator| propagator.inject_context(&cx, &mut carrier));
        TraceContext {
            traceparent: carrier.remove("traceparent").filter(|s| !s.is_empty()),
            tracestate: carrier.remove("tracestate").filter(|s| !s.is_empty()),
        }
    }
    #[cfg(not(feature = "tracing"))]
    {
        TraceContext::default()
    }
}

/// Parent `span` to the remote trace context carried by `tc` — the consumer calls
/// this on its handling span so the emitting span becomes its parent and the two
/// join one distributed trace.
///
/// With the `tracing` feature it extracts the context through the globally-installed
/// W3C propagator and sets it as the span's parent; without the feature it is a
/// generic no-op (it does not even name the `tracing` crate, so the facade compiles
/// with no tracing runtime). An empty `tc` is left untouched, so a span with no
/// incoming context keeps its natural local parent.
#[cfg(feature = "tracing")]
pub fn set_parent(span: &tracing::Span, tc: &TraceContext) {
    use opentelemetry::global;
    use std::collections::HashMap;
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    if tc.traceparent.is_none() {
        return;
    }
    let mut carrier: HashMap<String, String> = HashMap::new();
    if let Some(tp) = &tc.traceparent {
        carrier.insert("traceparent".to_string(), tp.clone());
    }
    if let Some(ts) = &tc.tracestate {
        carrier.insert("tracestate".to_string(), ts.clone());
    }
    let parent_cx = global::get_text_map_propagator(|propagator| propagator.extract(&carrier));
    // Best-effort: linking is observability, never the business path. It only errors
    // when no OTel layer is active for `span` (e.g. tracing on but no exporter), in
    // which case there is nothing to link and nothing to do.
    if let Err(err) = span.set_parent(parent_cx) {
        tracing::debug!(error = %err, "could not parent span to the incoming trace context");
    }
}

/// Parent a handling span to the remote trace context carried by `tc`. Without the
/// `tracing` feature this is a no-op generic over the span type, so the facade
/// links no tracing runtime; see the feature-enabled variant for the behaviour.
#[cfg(not(feature = "tracing"))]
pub fn set_parent<S: ?Sized>(_span: &S, _tc: &TraceContext) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_context_has_no_traceparent() {
        assert!(TraceContext::default().is_empty());
        assert!(!TraceContext {
            traceparent: Some("00-a-b-01".into()),
            tracestate: None,
        }
        .is_empty());
    }

    #[cfg(not(feature = "tracing"))]
    #[test]
    fn without_the_feature_injection_is_a_noop() {
        // The default build links no tracing runtime: capturing yields nothing and
        // parenting a span is inert.
        assert_eq!(inject_current(), TraceContext::default());
        let span = ();
        set_parent(&span, &TraceContext::default());
    }
}
