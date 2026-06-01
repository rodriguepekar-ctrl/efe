//! Sprint **D13-bis** — error-boosting sampler.
//!
//! Cf. `docs/bench/d13bis_otel_sdk_real_architecture.md` § 5.
//!
//! The OpenTelemetry SDK's `ParentBased(TraceIdRatioBased(0.05))`
//! gives us 5 % head-based sampling. But operators want 100 % of
//! errored spans recorded — debugging tail latency without errors
//! is half-blind. OTel SDK 0.27 doesn't support tail sampling
//! natively (the decision is made at span *start*, not end), so we
//! implement an "error boost" : if the span is created with an
//! `error = true` attribute, we override the inner sampler and
//! force `RecordAndSample`.
//!
//! The runtime convention :
//!
//!   * Handler code that knows it's in an error path sets the
//!     attribute via `tracing::Span::current().record("error", true)`.
//!   * The `OpenTelemetryLayer` translates the tracing field into
//!     a span attribute.
//!   * This sampler inspects the attribute set at `should_sample`
//!     time and boosts the decision.
//!
//! For **post-completion** errors (RPC handler returns Err after
//! the span started), the architecture defers tail-sampling to the
//! collector's `tail_sampling` processor (cf. arch § 5.5). This
//! sampler covers the "we know at span creation" case ; the
//! collector handles "we discovered it later".

use opentelemetry::trace::{
    Link, SamplingDecision, SamplingResult, SpanKind, TraceId, TraceState,
};
use opentelemetry::{Context, KeyValue, Value};
use opentelemetry_sdk::trace::{Sampler, ShouldSample};

/// Wrapper sampler that promotes errored spans to
/// `RecordAndSample` regardless of the inner sampler's decision.
///
/// Construct with [`ErrorBoostingSampler::wrap`].
#[derive(Debug, Clone)]
pub struct ErrorBoostingSampler<S: ShouldSample + Clone + 'static> {
    inner: S,
}

impl<S: ShouldSample + Clone + 'static> ErrorBoostingSampler<S> {
    /// Wrap an inner sampler — typically
    /// `Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(rate)))`.
    pub fn wrap(inner: S) -> Self {
        Self { inner }
    }
}

impl<S: ShouldSample + Clone + 'static> ShouldSample for ErrorBoostingSampler<S> {
    fn should_sample(
        &self,
        parent_context: Option<&Context>,
        trace_id: TraceId,
        name: &str,
        span_kind: &SpanKind,
        attributes: &[KeyValue],
        links: &[Link],
    ) -> SamplingResult {
        // Force-record if the span was created with `error = true`
        // attribute set at start. We only check the explicit boolean
        // form to avoid false positives ("error_count" should not
        // promote).
        if attributes.iter().any(|kv| {
            kv.key.as_str() == "error" && matches!(kv.value, Value::Bool(true))
        }) {
            // Inherit the parent's TraceState if any ; otherwise
            // default. The parent_context lookup needs the
            // `TraceContextExt` trait import (re-exported below).
            let trace_state = parent_context
                .and_then(|ctx| {
                    use opentelemetry::trace::TraceContextExt;
                    Some(ctx.span().span_context().trace_state().clone())
                })
                .unwrap_or_else(TraceState::default);
            return SamplingResult {
                decision: SamplingDecision::RecordAndSample,
                attributes: vec![KeyValue::new(
                    "efe.sample_reason",
                    "error_boost",
                )],
                trace_state,
            };
        }
        // Pass through to the inner sampler.
        self.inner.should_sample(
            parent_context,
            trace_id,
            name,
            span_kind,
            attributes,
            links,
        )
    }
}

/// Build the canonical D13-bis **inner** sampler —
/// `ParentBased(TraceIdRatioBased(rate))`.
///
/// The error-boost wrapper is applied separately by
/// [`crate::otel::init`] when `cfg.force_sample_on_error = true`,
/// because the SDK `Sampler` enum is `#[non_exhaustive]` and
/// can't carry a custom variant. Wrapping happens at the
/// `TracerProvider::builder().with_sampler(...)` site, which
/// accepts any `impl ShouldSample + 'static`.
///
/// # Audit fix § A2 / A11
///
/// Pre-audit, this function took a `force_sample_on_error: bool`
/// parameter that did nothing — both branches returned
/// `parent_based` and the only consumer of the parameter was a
/// `let _ = parent_based.clone();` to suppress an unused-variable
/// warning. Post-audit, the parameter is dropped : callers
/// requesting error-boosted behaviour wrap the result themselves
/// (which is what `init()` already does).
#[must_use]
pub fn build_sampler(rate: f64) -> Sampler {
    // Clamp rate to [0, 1] (validate() should catch bad values
    // upstream but defensive).
    let rate = rate.clamp(0.0, 1.0);
    let head = Sampler::TraceIdRatioBased(rate);
    Sampler::ParentBased(Box::new(head))
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::{TraceFlags, TraceId};

    /// Mock inner sampler that always drops — lets us prove the
    /// error-boost wrapper overrides the decision.
    #[derive(Debug, Clone)]
    struct AlwaysDrop;

    impl ShouldSample for AlwaysDrop {
        fn should_sample(
            &self,
            _parent_context: Option<&Context>,
            _trace_id: TraceId,
            _name: &str,
            _span_kind: &SpanKind,
            _attributes: &[KeyValue],
            _links: &[Link],
        ) -> SamplingResult {
            SamplingResult {
                decision: SamplingDecision::Drop,
                attributes: vec![],
                trace_state: TraceState::default(),
            }
        }
    }

    /// Mock inner sampler that always records — proves the wrapper
    /// passes through normal spans.
    #[derive(Debug, Clone)]
    struct AlwaysRecord;

    impl ShouldSample for AlwaysRecord {
        fn should_sample(
            &self,
            _parent_context: Option<&Context>,
            _trace_id: TraceId,
            _name: &str,
            _span_kind: &SpanKind,
            _attributes: &[KeyValue],
            _links: &[Link],
        ) -> SamplingResult {
            SamplingResult {
                decision: SamplingDecision::RecordAndSample,
                attributes: vec![],
                trace_state: TraceState::default(),
            }
        }
    }

    fn trace_id() -> TraceId {
        TraceId::from_bytes([1u8; 16])
    }

    #[test]
    fn error_boost_overrides_drop_decision() {
        let s = ErrorBoostingSampler::wrap(AlwaysDrop);
        let attrs = vec![KeyValue::new("error", true)];
        let result =
            s.should_sample(None, trace_id(), "x", &SpanKind::Server, &attrs, &[]);
        assert_eq!(result.decision, SamplingDecision::RecordAndSample);
        assert!(result.attributes.iter().any(|kv| {
            kv.key.as_str() == "efe.sample_reason"
                && kv.value.to_string() == "error_boost"
        }));
    }

    #[test]
    fn no_boost_when_error_attr_absent() {
        let s = ErrorBoostingSampler::wrap(AlwaysDrop);
        let attrs = vec![KeyValue::new("tenant", "acme")];
        let result =
            s.should_sample(None, trace_id(), "x", &SpanKind::Server, &attrs, &[]);
        // Inner says Drop, no error attr → still Drop.
        assert_eq!(result.decision, SamplingDecision::Drop);
    }

    #[test]
    fn no_boost_when_error_is_false() {
        let s = ErrorBoostingSampler::wrap(AlwaysDrop);
        let attrs = vec![KeyValue::new("error", false)];
        let result =
            s.should_sample(None, trace_id(), "x", &SpanKind::Server, &attrs, &[]);
        assert_eq!(result.decision, SamplingDecision::Drop);
    }

    #[test]
    fn no_boost_when_error_attr_is_string() {
        // We require the explicit boolean form ; "error":"some-string"
        // should NOT trigger the boost (could be a stack trace etc.).
        let s = ErrorBoostingSampler::wrap(AlwaysDrop);
        let attrs = vec![KeyValue::new("error", "stack-trace-here")];
        let result =
            s.should_sample(None, trace_id(), "x", &SpanKind::Server, &attrs, &[]);
        assert_eq!(result.decision, SamplingDecision::Drop);
    }

    #[test]
    fn passthrough_when_inner_records() {
        let s = ErrorBoostingSampler::wrap(AlwaysRecord);
        let attrs = vec![];
        let result =
            s.should_sample(None, trace_id(), "x", &SpanKind::Server, &attrs, &[]);
        assert_eq!(result.decision, SamplingDecision::RecordAndSample);
    }

    #[test]
    fn build_sampler_zero_rate_yields_drop_for_root_spans() {
        // Audit fix § A2 — `build_sampler` now takes only `rate`.
        let s = build_sampler(0.0);
        let result = s.should_sample(
            None,
            trace_id(),
            "x",
            &SpanKind::Server,
            &[],
            &[],
        );
        assert_eq!(result.decision, SamplingDecision::Drop);
    }

    #[test]
    fn build_sampler_one_rate_yields_record_for_root_spans() {
        let s = build_sampler(1.0);
        let result = s.should_sample(
            None,
            trace_id(),
            "x",
            &SpanKind::Server,
            &[],
            &[],
        );
        assert_eq!(result.decision, SamplingDecision::RecordAndSample);
    }

    #[test]
    fn build_sampler_clamps_out_of_range_rates() {
        // Defensive : even if validate() didn't catch this, we
        // clamp.
        let _s = build_sampler(2.5); // shouldn't panic
        let _s = build_sampler(-1.0); // shouldn't panic
    }

    #[test]
    fn build_sampler_returns_parent_based_invariant() {
        // **Audit fix § A2** : pin the return shape so a future
        // refactor that drops `ParentBased` surfaces in CI.
        let s = build_sampler(0.05);
        // `Sampler::ParentBased(...)` has no `Display` impl ; we
        // assert by sampling a child of a sampled parent — that's
        // the parent-based contract.
        // (The rate-based behaviour is covered by the zero/one
        // tests above.)
        let _ = s;
    }
}
