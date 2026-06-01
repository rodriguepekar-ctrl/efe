//! PR #18 § 16.2 / AD8 — `always_on_error` sampler overlay.
//!
//! Wraps an inner [`Sampler`] (typically `TraceIdRatioBased(rate)`) and
//! force-samples any span whose creation-time attributes indicate an
//! error condition.
//!
//! ## Why head-sampling errors
//!
//! Tail-based sampling at the collector layer is the canonical way to
//! retain error traces (T61 mitigation).  But tail-sampling requires
//! the collector to buffer entire traces in memory until the trace
//! ends, which is expensive at scale.
//!
//! For the high-value subset of traces — those that already fail at
//! span creation time (e.g. `Status::DeadlineExceeded`, throttling
//! responses, validation errors) — we cheat : we look for an
//! `error=true` attribute on the proposed span and force the
//! `RecordAndSample` decision before the sampler ever runs.  This
//! makes the head sampler **always_on_error-aware** even before the
//! span has accumulated any payload.
//!
//! For traces whose errors only surface mid-span (e.g. the gRPC
//! response is `Err` set in the close handler), the head sampler's
//! decision has already been made.  These rely on the collector's
//! tail-sampling policy.

use opentelemetry::trace::{Link, SamplingDecision, SamplingResult, SpanKind, TraceId, TraceState};
use opentelemetry::KeyValue;
use opentelemetry_sdk::trace::{Sampler, ShouldSample};

/// Sampler that wraps an inner sampler and force-samples spans whose
/// creation-time attributes contain `error=true`.
#[derive(Debug, Clone)]
pub struct AlwaysOnErrorSampler {
    inner: Sampler,
}

impl AlwaysOnErrorSampler {
    /// Wrap an inner sampler.  Typical usage : the inner is
    /// `Sampler::TraceIdRatioBased(rate)`.
    #[must_use]
    pub fn new(inner: Sampler) -> Self {
        Self { inner }
    }
}

impl ShouldSample for AlwaysOnErrorSampler {
    fn should_sample(
        &self,
        parent_context: Option<&opentelemetry::Context>,
        trace_id: TraceId,
        name: &str,
        span_kind: &SpanKind,
        attributes: &[KeyValue],
        links: &[Link],
    ) -> SamplingResult {
        // ── 1. Force-sample if `error=true` is present ──
        if attributes.iter().any(|kv| {
            kv.key.as_str() == "error" && matches!(&kv.value, opentelemetry::Value::Bool(b) if *b)
        }) {
            return SamplingResult {
                decision: SamplingDecision::RecordAndSample,
                attributes: vec![KeyValue::new(
                    "meta_swarm.sampling.decision",
                    "forced_on_error",
                )],
                trace_state: TraceState::default(),
            };
        }

        // ── 2. Force-sample if `meta_swarm.iterate.outcome` ∈ error class ──
        if attributes.iter().any(|kv| {
            kv.key.as_str() == "meta_swarm.iterate.outcome"
                && matches!(
                    &kv.value,
                    opentelemetry::Value::String(s)
                        if matches!(s.as_str(), "error" | "deadline" | "throttled")
                )
        }) {
            return SamplingResult {
                decision: SamplingDecision::RecordAndSample,
                attributes: vec![KeyValue::new(
                    "meta_swarm.sampling.decision",
                    "forced_on_error",
                )],
                trace_state: TraceState::default(),
            };
        }

        // ── 3. Delegate to the inner sampler ──
        let mut result =
            self.inner
                .should_sample(parent_context, trace_id, name, span_kind, attributes, links);
        // Annotate the decision for downstream tooling.
        result.attributes.push(KeyValue::new(
            "meta_swarm.sampling.decision",
            match result.decision {
                SamplingDecision::RecordAndSample => "sampled",
                SamplingDecision::RecordOnly => "recorded_only",
                SamplingDecision::Drop => "dropped",
            },
        ));
        result
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ratio_sampler(rate: f64) -> Sampler {
        Sampler::TraceIdRatioBased(rate)
    }

    fn always_off() -> Sampler {
        Sampler::AlwaysOff
    }

    #[test]
    fn explicit_error_attribute_forces_sample() {
        let s = AlwaysOnErrorSampler::new(always_off());
        let result = s.should_sample(
            None,
            TraceId::from_bytes([0; 16]),
            "test",
            &SpanKind::Server,
            &[KeyValue::new("error", true)],
            &[],
        );
        assert_eq!(result.decision, SamplingDecision::RecordAndSample);
        let tag = result
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "meta_swarm.sampling.decision")
            .expect("annotation present");
        assert!(
            matches!(&tag.value, opentelemetry::Value::String(s) if s.as_str() == "forced_on_error")
        );
    }

    #[test]
    fn iterate_outcome_error_forces_sample() {
        let s = AlwaysOnErrorSampler::new(always_off());
        let result = s.should_sample(
            None,
            TraceId::from_bytes([0; 16]),
            "test",
            &SpanKind::Server,
            &[KeyValue::new("meta_swarm.iterate.outcome", "error")],
            &[],
        );
        assert_eq!(result.decision, SamplingDecision::RecordAndSample);
    }

    #[test]
    fn iterate_outcome_deadline_forces_sample() {
        let s = AlwaysOnErrorSampler::new(always_off());
        let result = s.should_sample(
            None,
            TraceId::from_bytes([0; 16]),
            "test",
            &SpanKind::Server,
            &[KeyValue::new("meta_swarm.iterate.outcome", "deadline")],
            &[],
        );
        assert_eq!(result.decision, SamplingDecision::RecordAndSample);
    }

    #[test]
    fn no_error_delegates_to_inner_drop() {
        let s = AlwaysOnErrorSampler::new(always_off());
        let result = s.should_sample(
            None,
            TraceId::from_bytes([0; 16]),
            "test",
            &SpanKind::Server,
            &[KeyValue::new("meta_swarm.iterate.outcome", "ok")],
            &[],
        );
        assert_eq!(result.decision, SamplingDecision::Drop);
    }

    #[test]
    fn no_error_delegates_to_inner_with_rate() {
        // TraceIdRatioBased(1.0) is deterministic — always RecordAndSample.
        let s = AlwaysOnErrorSampler::new(ratio_sampler(1.0));
        let result = s.should_sample(
            None,
            TraceId::from_bytes([0xDE, 0xAD, 0xBE, 0xEF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            "test",
            &SpanKind::Server,
            &[],
            &[],
        );
        assert_eq!(result.decision, SamplingDecision::RecordAndSample);
        let tag = result
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "meta_swarm.sampling.decision")
            .expect("annotation present");
        assert!(matches!(&tag.value, opentelemetry::Value::String(s) if s.as_str() == "sampled"));
    }

    #[test]
    fn error_attribute_false_does_not_force() {
        let s = AlwaysOnErrorSampler::new(always_off());
        let result = s.should_sample(
            None,
            TraceId::from_bytes([0; 16]),
            "test",
            &SpanKind::Server,
            &[KeyValue::new("error", false)],
            &[],
        );
        // `error=false` is benign — delegate to inner.
        assert_eq!(result.decision, SamplingDecision::Drop);
    }

    #[test]
    fn outcome_ok_does_not_force() {
        let s = AlwaysOnErrorSampler::new(always_off());
        let result = s.should_sample(
            None,
            TraceId::from_bytes([0; 16]),
            "test",
            &SpanKind::Server,
            &[KeyValue::new("meta_swarm.iterate.outcome", "ok")],
            &[],
        );
        assert_eq!(result.decision, SamplingDecision::Drop);
    }
}
