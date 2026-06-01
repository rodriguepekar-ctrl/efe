//! **Sprint M4 audit fixes S-A2 / S-A3** — synthetic SwarmSpec
//! construction shared by Mode-2 (D34) and Mode-3 (D37).
//!
//! ## Why this module exists
//!
//! The Mode-2 and Mode-3 services receive a free-form `query_text`
//! plus auxiliary metadata (router output for Mode-2, k for Mode-3).
//! Pre-audit both services SYNTHESISED a verdict by reading
//! `router_output.anchors[0].confidence` (Mode-2) or by computing
//! `0.5 + idx*0.1` (Mode-3) — the swarm controller was NEVER invoked.
//!
//! This is a Sprint-M4 audit defect (M4-A2 / M4-A3). The fix routes
//! every Mode-2 / Mode-3 sub-call through the shared
//! `DynExecuteBackend` so :
//!
//!   1. Telemetry, rate limiting, idempotency, audit emission all see
//!      the load (per-sub-swarm + per-mode-2 call counters).
//!   2. The verdict is REAL — produced by the actual `SwarmController`
//!      pipeline (genesis → fusion → consensus → dissolution → align).
//!   3. The pin tests can grep for `backend.execute(` to detect a
//!      regression that would silently revert to synthesis.
//!
//! ## Synthesis dimension
//!
//! We use a fixed 8-dimensional latent space for the synthetic spec.
//! 8 is large enough to exercise the SIMD paths in
//! `efe-core::compute` (AVX2 / NEON kernels operate on 8-wide chunks)
//! while keeping the per-call wall time negligible (< 1 ms on the
//! MVP CPU backend).
//!
//! Future work (`DT-S-D38-*`) replaces this synthesis with a real
//! catalog-bound spec that incorporates Mode-2's dynamic anchors and
//! Mode-3's per-sub-swarm seed perturbation.

use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};
use efe_core::{Request, RequestId};
use sha2::{Digest, Sha256};
use tonic::Status;

/// Synthesis latent dimension. Cf. module doc.
pub const SYNTH_DIM: usize = 8;

/// Default population for the synthetic spec : 4 NaSem agents.
/// Small enough to keep latency bounded ; non-trivial enough to
/// exercise the consensus detector with > 1 vote.
pub const SYNTH_POPULATION: u32 = 4;

/// Default max rounds for the synthetic spec.
pub const SYNTH_MAX_ROUNDS: u32 = 3;

/// Default cone radius for the synthetic spec.
pub const SYNTH_CONE_RADIUS: f32 = 0.15;

/// Build a deterministic [`Request`] from a free-form query.
///
/// The generated `problem_embedding` is derived from
/// `SHA-256(query || seed)`, so two calls with the same `(query, seed)`
/// produce identical specs (modulo `request_id`). This satisfies the
/// "same input → same output" determinism contract documented in the
/// SLO budget S-7.
///
/// # Errors
///
/// Returns [`tonic::Status::Internal`] if any of the validated
/// constructors (`LatentDim::new`, `PopulationMix::new`, `AlignmentCone::new`,
/// `SwarmSpec::builder`) reject the synthesised inputs. In practice
/// none of them can fail because the constants above are pre-validated.
/// The error path exists so a future code change to the constants
/// (e.g. `SYNTH_DIM = 0`) surfaces at runtime rather than panicking.
pub fn build_synthetic_request(query: &str, seed: u64, tenant_id: &str) -> Result<Request, Status> {
    let problem_embedding = derive_embedding(query, seed);
    let dim = LatentDim::new(SYNTH_DIM as u32)
        .map_err(|e| Status::internal(format!("LatentDim::new({SYNTH_DIM}): {e}")))?;
    let pop = PopulationMix::new(vec![(TemplateCode::NaSem, SYNTH_POPULATION)])
        .map_err(|e| Status::internal(format!("PopulationMix::new: {e}")))?;
    let cone = AlignmentCone::new(vec![vec![0.1_f32; SYNTH_DIM]], SYNTH_CONE_RADIUS)
        .map_err(|e| Status::internal(format!("AlignmentCone::new: {e}")))?;

    let spec = SwarmSpec::builder()
        .problem_embedding(problem_embedding)
        .dim_latent(dim)
        .population(pop)
        .max_rounds(SYNTH_MAX_ROUNDS)
        .map_err(|e| Status::internal(format!("max_rounds({SYNTH_MAX_ROUNDS}): {e}")))?
        .alignment_cone(cone)
        .build()
        .map_err(|e| Status::internal(format!("SwarmSpec::builder().build(): {e}")))?;

    Ok(Request::synthetic(
        spec,
        tenant_id.to_string(),
        RequestId::new(),
    ))
}

/// Derive an `SYNTH_DIM`-element f32 vector from
/// `SHA-256(query || seed)`. Each f32 lives in `[-1, 1]`.
///
/// We chain the digest until we have enough bytes (32-byte SHA-256 →
/// 8 f32 — exactly enough at SYNTH_DIM = 8).
fn derive_embedding(query: &str, seed: u64) -> Vec<f32> {
    let mut hasher = Sha256::new();
    hasher.update(query.as_bytes());
    hasher.update(seed.to_le_bytes());
    let h = hasher.finalize();
    let mut v = Vec::with_capacity(SYNTH_DIM);
    for i in 0..SYNTH_DIM {
        let b = i * 4;
        let raw = i32::from_le_bytes([h[b], h[b + 1], h[b + 2], h[b + 3]]);
        v.push((raw as f32) / (i32::MAX as f32));
    }
    debug_assert_eq!(v.len(), SYNTH_DIM);
    v
}

/// Translate a `efe_core::Verdict` (rich enum) into the proto
/// `efe.v1.Verdict` int representation. Mirror of
/// `crates/efe-server/src/bridge/verdict.rs::to_proto` but returns the
/// i32 directly (Mode-2/3 protos use the bare int field).
#[must_use]
pub fn verdict_to_proto_i32(v: &efe_core::Verdict) -> i32 {
    use crate::proto::v1::Verdict as ProtoVerdict;
    match v {
        efe_core::Verdict::Continue => ProtoVerdict::Continue as i32,
        efe_core::Verdict::Converged { .. } => ProtoVerdict::Converged as i32,
        efe_core::Verdict::Divergence { .. } => ProtoVerdict::Divergence as i32,
        efe_core::Verdict::ModeCollapse => ProtoVerdict::Converged as i32, // mapped to Converged
                                                                           // intentionally — mode collapse
                                                                           // ends the swarm with a final
                                                                           // (low-confidence) verdict.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_synthetic_request_deterministic() {
        let r1 = build_synthetic_request("hello", 42, "tenant-a").unwrap();
        let r2 = build_synthetic_request("hello", 42, "tenant-a").unwrap();
        // request_id differs (UUID v7) but the spec is deterministic.
        assert_eq!(r1.spec.problem_embedding, r2.spec.problem_embedding);
        assert_eq!(r1.spec.dim_latent.get(), SYNTH_DIM as u32);
        assert_eq!(r1.tenant_id, r2.tenant_id);
        assert_eq!(r1.tenant_id, "tenant-a");
    }

    #[test]
    fn build_synthetic_request_query_changes_spec() {
        let a = build_synthetic_request("query A", 0, "t").unwrap();
        let b = build_synthetic_request("query B", 0, "t").unwrap();
        assert_ne!(a.spec.problem_embedding, b.spec.problem_embedding);
    }

    #[test]
    fn build_synthetic_request_seed_changes_spec() {
        let a = build_synthetic_request("q", 0, "t").unwrap();
        let b = build_synthetic_request("q", 1, "t").unwrap();
        assert_ne!(a.spec.problem_embedding, b.spec.problem_embedding);
    }

    #[test]
    fn derive_embedding_in_range() {
        let v = derive_embedding("anything", 123);
        assert_eq!(v.len(), SYNTH_DIM);
        for x in v {
            assert!(x >= -1.0 && x <= 1.0, "f32 out of range: {x}");
        }
    }

    #[test]
    fn verdict_to_proto_i32_known_values() {
        assert_eq!(
            verdict_to_proto_i32(&efe_core::Verdict::Continue),
            crate::proto::v1::Verdict::Continue as i32,
        );
        assert_eq!(
            verdict_to_proto_i32(&efe_core::Verdict::Converged { confidence: 0.9 }),
            crate::proto::v1::Verdict::Converged as i32,
        );
        assert_eq!(
            verdict_to_proto_i32(&efe_core::Verdict::Divergence { reason: "x" }),
            crate::proto::v1::Verdict::Divergence as i32,
        );
    }
}
