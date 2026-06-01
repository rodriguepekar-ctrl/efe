//! PR #18 § 14 — W3C Trace Context propagator.
//!
//! Wraps the OTel SDK's [`opentelemetry_sdk::propagation::TraceContextPropagator`]
//! and adapts it to `tonic::metadata::MetadataMap` via two small
//! adapter types (`TonicMetadataInjector` + `TonicMetadataExtractor`).
//!
//! Wire format : W3C `traceparent` + `tracestate` HTTP headers.
//! gRPC carries them as ASCII metadata entries.

use opentelemetry::propagation::{Extractor, Injector, TextMapPropagator};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tonic::metadata::{MetadataKey, MetadataMap, MetadataValue};

/// Thin alias re-export of the W3C-spec propagator.  Re-exported here
/// so callers in the binary don't pull a direct `opentelemetry_sdk`
/// dep.
pub type W3CTraceContextPropagator = TraceContextPropagator;

// ─────────────────────────────────────────────────────────────────────
// Injector / Extractor adapters for tonic MetadataMap.
// ─────────────────────────────────────────────────────────────────────

/// Adapter implementing [`Injector`] over a mutable [`MetadataMap`].
///
/// Writes `traceparent` / `tracestate` ASCII-safe headers ; non-ASCII
/// values are dropped with a tracing warn.
#[derive(Debug)]
pub struct TonicMetadataInjector<'a>(pub &'a mut MetadataMap);

impl Injector for TonicMetadataInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        // ASCII metadata values are mandatory per W3C ; non-ASCII would
        // produce a binary-metadata entry which downstream OTel
        // collectors interpret unpredictably.  Reject up front.
        if !value.is_ascii() {
            tracing::warn!(
                ?key,
                %value,
                "OTel propagator : non-ASCII header value dropped",
            );
            return;
        }
        let Ok(name) = MetadataKey::from_bytes(key.as_bytes()) else {
            tracing::warn!(?key, "OTel propagator : invalid header name");
            return;
        };
        let Ok(val) = MetadataValue::try_from(value.as_str()) else {
            tracing::warn!(?key, %value, "OTel propagator : metadata value rejected");
            return;
        };
        self.0.insert(name, val);
    }
}

/// Adapter implementing [`Extractor`] over a read-only [`MetadataMap`].
#[derive(Debug)]
pub struct TonicMetadataExtractor<'a>(pub &'a MetadataMap);

impl Extractor for TonicMetadataExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .filter_map(|k| match k {
                tonic::metadata::KeyRef::Ascii(k) => Some(k.as_str()),
                tonic::metadata::KeyRef::Binary(_) => None,
            })
            .collect()
    }
}

// ─────────────────────────────────────────────────────────────────────
// Convenience helpers used by the interceptors.
// ─────────────────────────────────────────────────────────────────────

/// Inject the current OTel context into the supplied [`MetadataMap`].
/// Used by the client interceptor (controller → participant outbound
/// gRPC calls).
pub fn inject_current_context(propagator: &W3CTraceContextPropagator, metadata: &mut MetadataMap) {
    let cx = tracing::Span::current().context();
    let mut injector = TonicMetadataInjector(metadata);
    propagator.inject_context(&cx, &mut injector);
}

/// Extract the parent OTel context from the supplied [`MetadataMap`].
/// Used by the server interceptor (controller + participant inbound
/// gRPC calls).
pub fn extract_parent_context(
    propagator: &W3CTraceContextPropagator,
    metadata: &MetadataMap,
) -> opentelemetry::Context {
    let extractor = TonicMetadataExtractor(metadata);
    propagator.extract(&extractor)
}

// ─────────────────────────────────────────────────────────────────────
// re-export glue : tracing-opentelemetry bridge.  See lib.rs for use.
// ─────────────────────────────────────────────────────────────────────

/// Convenience trait re-export so callers can write
/// `span.context()` / `span.set_parent(cx)` without importing
/// `tracing_opentelemetry::OpenTelemetrySpanExt` themselves.
pub use tracing_opentelemetry::OpenTelemetrySpanExt;

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
    use opentelemetry::Context;

    #[test]
    fn injector_writes_traceparent_to_metadata() {
        let mut md = MetadataMap::new();
        let mut inj = TonicMetadataInjector(&mut md);
        inj.set(
            "traceparent",
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01".to_string(),
        );
        assert_eq!(
            md.get("traceparent").unwrap().to_str().unwrap(),
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
        );
    }

    #[test]
    fn extractor_reads_traceparent() {
        let mut md = MetadataMap::new();
        md.insert(
            "traceparent",
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
                .parse()
                .unwrap(),
        );
        let ext = TonicMetadataExtractor(&md);
        assert_eq!(
            ext.get("traceparent"),
            Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")
        );
    }

    #[test]
    fn extractor_returns_none_for_missing_key() {
        let md = MetadataMap::new();
        let ext = TonicMetadataExtractor(&md);
        assert_eq!(ext.get("nope"), None);
    }

    #[test]
    fn roundtrip_via_propagator_preserves_trace_id() {
        let propagator = W3CTraceContextPropagator::new();

        // Build a parent context with a known trace_id + span_id.
        let trace_id = TraceId::from_bytes([
            0x0a, 0xf7, 0x65, 0x19, 0x16, 0xcd, 0x43, 0xdd, 0x84, 0x48, 0xeb, 0x21, 0x1c, 0x80,
            0x31, 0x9c,
        ]);
        let span_id = SpanId::from_bytes([0xb7, 0xad, 0x6b, 0x71, 0x69, 0x20, 0x33, 0x31]);
        let span_ctx = SpanContext::new(
            trace_id,
            span_id,
            TraceFlags::SAMPLED,
            true, // remote
            TraceState::default(),
        );
        use opentelemetry::trace::TraceContextExt;
        let parent_cx = Context::current().with_remote_span_context(span_ctx);

        // Inject.
        let mut md = MetadataMap::new();
        {
            let mut inj = TonicMetadataInjector(&mut md);
            propagator.inject_context(&parent_cx, &mut inj);
        }
        let tp = md.get("traceparent").expect("traceparent injected");
        assert!(tp
            .to_str()
            .unwrap()
            .contains("0af7651916cd43dd8448eb211c80319c"));

        // Extract.
        let extracted = propagator.extract(&TonicMetadataExtractor(&md));
        let extracted_trace_id = extracted.span().span_context().trace_id();
        assert_eq!(extracted_trace_id, trace_id);
    }

    #[test]
    fn injector_drops_non_ascii_value() {
        let mut md = MetadataMap::new();
        let mut inj = TonicMetadataInjector(&mut md);
        // The traceparent value MUST be ASCII per W3C.  Non-ASCII is
        // dropped silently (with a tracing warn we can't easily assert
        // on in unit tests).
        inj.set("traceparent", "héllo".to_string());
        assert!(md.get("traceparent").is_none());
    }
}
