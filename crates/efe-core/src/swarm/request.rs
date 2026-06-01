//! Request and Response types carried through the controller.
//!
//! These types form the public data contract between M10 (API layer) and
//! M1 (core runtime). They are versioned through the gRPC / SDK layer
//! and kept deliberately minimal in MVP.

use std::num::NonZeroU32;
use std::time::Instant;

use crate::swarm::verdict::Verdict;
use crate::types::{RequestId, SwarmId, SwarmSpec};

/// Inference request entering the core runtime.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Request {
    /// Propagated from the API layer for correlation.
    pub id: RequestId,
    /// Tenant identifier (enforced by M9 rate limiter before arrival).
    pub tenant_id: String,
    /// Full swarm specification.
    pub spec: SwarmSpec,
    /// How to decode the final centroid.
    pub decode_mode: DecodeMode,
    /// Optional raw prompt (audit only — not used for computation once
    /// the embedding is produced).
    pub raw_prompt: Option<String>,
    /// Optional absolute deadline. If `Some`, the arbiter clamps the
    /// effective budget to `deadline - now`.
    pub deadline: Option<Instant>,
    /// W3C Trace Context (traceparent header) for distributed tracing.
    pub trace_context: Option<TraceContext>,
}

impl Request {
    /// Build a minimal test request with sensible defaults.
    #[must_use]
    #[doc(hidden)]
    pub fn for_tests(spec: SwarmSpec) -> Self {
        Self {
            id: RequestId::new(),
            tenant_id: "test".into(),
            spec,
            decode_mode: DecodeMode::PassThrough,
            raw_prompt: None,
            deadline: None,
            trace_context: None,
        }
    }

    /// Public constructor for synthetic requests built server-side.
    ///
    /// **Sprint M4 audit fix S-A3** — `efe-server`'s Mode-2 and Mode-3
    /// services need to invoke the backend with a `Request` that
    /// wasn't constructed from a proto `ExecuteRequest` (the mode-N
    /// callers don't have one — they have a `query_text` and a router
    /// output). Pre-fix they would have used `Request::for_tests`
    /// which is `#[doc(hidden)]` and stamps `tenant_id = "test"`
    /// regardless of the real caller. This constructor is the stable,
    /// public path :
    ///
    /// * `spec` — the [`SwarmSpec`] the caller built from its inputs.
    /// * `tenant_id` — propagated for telemetry + audit attribution.
    /// * `request_id` — server-assigned correlation id.
    ///
    /// `decode_mode` defaults to [`DecodeMode::PassThrough`] (Mode-2/3
    /// callers post-process the centroid themselves).
    #[must_use]
    pub fn synthetic(spec: SwarmSpec, tenant_id: String, request_id: RequestId) -> Self {
        Self {
            id: request_id,
            tenant_id,
            spec,
            decode_mode: DecodeMode::PassThrough,
            raw_prompt: None,
            deadline: None,
            trace_context: None,
        }
    }
}

/// Output decoding mode.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum DecodeMode {
    /// Text decoding via LM head (default for chat/NL queries).
    Text {
        /// Maximum number of tokens to emit.
        max_tokens: NonZeroU32,
    },
    /// Return the raw centroid for downstream chaining.
    PassThrough,
}

impl Default for DecodeMode {
    fn default() -> Self {
        Self::Text {
            max_tokens: NonZeroU32::new(256).expect("256 is non-zero"),
        }
    }
}

/// W3C Trace Context header (simplified).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceContext {
    /// The `traceparent` header value.
    pub traceparent: String,
    /// Optional `tracestate`.
    pub tracestate: Option<String>,
}

/// Response produced by the core runtime.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Response {
    /// Swarm id (correlate with telemetry).
    pub swarm_id: SwarmId,
    /// Echo of the request id.
    pub request_id: RequestId,
    /// Decoded output.
    pub output: DecodedOutput,
    /// Confidence at consensus, `[0, 1]`.
    pub confidence: f32,
    /// Final verdict.
    pub verdict: Verdict,
    /// Number of rounds actually executed.
    pub rounds_executed: u32,
    /// Total latency of the swarm, in milliseconds.
    pub total_ms: f32,
    /// Alignment projection distance (`0` if no projection applied).
    pub alignment_delta: f32,
}

impl Response {
    /// Minimal response builder intended for tests and stub backends.
    ///
    /// Callers are expected to overwrite any field that matters for
    /// their scenario via the public fields — the struct is
    /// `#[non_exhaustive]` and cannot be constructed by literal
    /// outside this crate.
    #[must_use]
    #[doc(hidden)]
    pub fn for_tests(request_id: RequestId, output: DecodedOutput) -> Self {
        Self {
            swarm_id: SwarmId::new(),
            request_id,
            output,
            confidence: 1.0,
            verdict: Verdict::Converged { confidence: 1.0 },
            rounds_executed: 1,
            total_ms: 0.0,
            alignment_delta: 0.0,
        }
    }
}

/// Decoded output produced by the dissolver.
#[derive(Clone, Debug, PartialEq)]
pub enum DecodedOutput {
    /// Text output (tokens already detokenized).
    Text(String),
    /// Raw centroid (when [`DecodeMode::PassThrough`] is used).
    Raw(Vec<f32>),
}

impl DecodedOutput {
    /// Borrow as text, if available.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            Self::Raw(_) => None,
        }
    }

    /// Borrow as a raw centroid, if available.
    #[must_use]
    pub fn as_raw(&self) -> Option<&[f32]> {
        match self {
            Self::Raw(v) => Some(v),
            Self::Text(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};

    fn sample_spec() -> SwarmSpec {
        SwarmSpec::builder()
            .problem_embedding(vec![0.0; 4])
            .dim_latent(LatentDim::new(4).expect("valid"))
            .population(PopulationMix::new(vec![(TemplateCode::NaSem, 4)]).expect("valid"))
            .max_rounds(3)
            .expect("valid rounds")
            .alignment_cone(AlignmentCone::new(vec![vec![0.1; 4]], 0.15).expect("valid cone"))
            .build()
            .expect("valid spec")
    }

    #[test]
    fn request_for_tests_is_minimal() {
        let req = Request::for_tests(sample_spec());
        assert_eq!(req.tenant_id, "test");
        assert!(req.raw_prompt.is_none());
        assert!(req.deadline.is_none());
    }

    #[test]
    fn decode_mode_default_is_text() {
        let dm = DecodeMode::default();
        assert!(matches!(dm, DecodeMode::Text { .. }));
    }

    #[test]
    fn decoded_output_views() {
        let text = DecodedOutput::Text("hello".into());
        assert_eq!(text.as_text(), Some("hello"));
        assert!(text.as_raw().is_none());

        let raw = DecodedOutput::Raw(vec![1.0, 2.0, 3.0]);
        assert_eq!(raw.as_raw(), Some(&[1.0f32, 2.0, 3.0][..]));
        assert!(raw.as_text().is_none());
    }
}
