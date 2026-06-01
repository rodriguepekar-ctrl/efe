//! **Sprint M1 extension D34** — Synthetic-spec construction.
//!
//! ## Backport
//!
//! This file is a backport of
//! `crates/efe-server/src/services/synthetic_spec.rs` (delivered in
//! M4 audit pass 1, fix S-A2/S-A3) into `efe-core`. The logic is
//! preserved byte-for-byte ; only the path changes.
//!
//! Once D34 ships, the `efe-server` copy will be deleted in favour
//! of `pub use efe_core::swarm::mode2::synthesis::*`.
//!
//! ## Mission
//!
//! Mode-2 (D34) and Mode-3 (D37) receive a free-form `query_text`
//! plus auxiliary metadata. They need to invoke the
//! [`crate::swarm::backend::ExecuteBackend`] which expects a typed
//! [`crate::Request`] containing a [`crate::types::SwarmSpec`].
//!
//! This module provides the deterministic bridge :
//!
//!   `(query_text, seed) → Request`
//!
//! The generated `problem_embedding` is derived from
//! `SHA-256(query || seed)`, so two calls with the same inputs
//! produce identical specs (modulo `request_id`).
//!
//! ## Synthesis dimension
//!
//! Fixed at 8 — large enough to exercise the SIMD paths in
//! `efe-core::compute` (AVX2 / NEON kernels operate on 8-wide
//! chunks) while keeping the per-call wall time negligible (< 1 ms
//! on the MVP CPU backend).
//!
//! Future work (`DT-C-D38-MODE2-1`) replaces this synthesis with a
//! real catalog-bound spec that incorporates Mode-2's dynamic
//! anchors. Until then, the synthesis exercises the backend
//! pipeline end-to-end.

use sha2::{Digest, Sha256};

use crate::errors::{EfeError, Result};
use crate::types::{
    AlignmentCone, LatentDim, PopulationMix, RequestId, SwarmSpec, TemplateCode,
};
use crate::Request;

/// Synthesis latent dimension. Cf. module doc.
pub const SYNTH_DIM: usize = 8;

/// Default population for the synthetic spec : 4 NaSem agents.
/// Small enough to keep latency bounded ; non-trivial enough to
/// exercise the consensus detector with > 1 vote.
pub const SYNTH_POPULATION: u32 = 4;

/// Default max rounds for the synthetic spec.
pub const SYNTH_MAX_ROUNDS: u32 = 3;

/// Default cone radius for the synthetic spec. 0.15 = lenient
/// enough that any reasonable centroid stays inside the cone, but
/// strict enough to catch genuinely-divergent runs.
pub const SYNTH_CONE_RADIUS: f32 = 0.15;

/// Build a deterministic [`Request`] from a free-form query.
///
/// **Determinism contract** : `(query, seed)` ⇒ same
/// `problem_embedding`, same `dim_latent`, same `population`. The
/// `request_id` is regenerated each call (UUID v7) so audit logs
/// can correlate per-call but the swarm trajectory is reproducible.
///
/// # Errors
///
/// Returns [`EfeError::InvalidConfig`] if any of the validated
/// constructors (`LatentDim::new`, `PopulationMix::new`, …) reject
/// the synthesised inputs. In practice none of them can fail
/// because the constants above are pre-validated.
pub fn build_synthetic_request(query: &str, seed: u64, tenant_id: &str) -> Result<Request> {
    let problem_embedding = derive_embedding(query, seed);
    let dim = LatentDim::new(SYNTH_DIM as u32).map_err(|e| {
        EfeError::InvalidConfig(format!("LatentDim::new({SYNTH_DIM}): {e}"))
    })?;
    let pop = PopulationMix::new(vec![(TemplateCode::NaSem, SYNTH_POPULATION)])
        .map_err(|e| EfeError::InvalidConfig(format!("PopulationMix::new: {e}")))?;
    let cone = AlignmentCone::new(vec![vec![0.1_f32; SYNTH_DIM]], SYNTH_CONE_RADIUS)
        .map_err(|e| EfeError::InvalidConfig(format!("AlignmentCone::new: {e}")))?;

    let spec = SwarmSpec::builder()
        .problem_embedding(problem_embedding)
        .dim_latent(dim)
        .population(pop)
        .max_rounds(SYNTH_MAX_ROUNDS)
        .map_err(|e| EfeError::InvalidConfig(format!("max_rounds({SYNTH_MAX_ROUNDS}): {e}")))?
        .alignment_cone(cone)
        .build()
        .map_err(|e| EfeError::InvalidConfig(format!("SwarmSpec::builder().build(): {e}")))?;

    Ok(Request::synthetic(
        spec,
        tenant_id.to_string(),
        RequestId::new(),
    ))
}

/// Derive an `SYNTH_DIM`-element f32 vector from
/// `SHA-256(query || seed)`. Each f32 lives in `[-1, 1]`.
///
/// **Algorithm** : 32-byte SHA-256 digest → 8 i32 little-endian
/// words → cast to f32 in [-1, 1] via `i32::MAX` normalisation.
/// Exactly enough material for `SYNTH_DIM = 8` ; if SYNTH_DIM
/// were larger we'd chain `seed = SHA256(seed)` until we have
/// enough bytes.
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

/// Translate a [`crate::Verdict`] (rich enum) into a stable u8
/// representation matching the proto `efe.v1.Verdict` enum values :
///
/// | Domain variant        | proto         | u8 |
/// |-----------------------|---------------|---:|
/// | (none)                | UNSPECIFIED   | 0  |
/// | `Continue`            | CONTINUE      | 1  |
/// | `Converged { ... }`   | CONVERGED     | 2  |
/// | `Divergence { ... }`  | DIVERGENCE    | 3  |
/// | `ModeCollapse`        | MODE_COLLAPSE | 4  |
///
/// **Audit fix C-A3** — pre-fix `ModeCollapse` was mapped to `2`
/// (Converged) on the rationale that "the collapse-engine's
/// centroid_project strategy turns it into a Converged verdict by
/// the time it leaves the controller". This was incorrect for two
/// reasons :
///
///   1. The proto enum **does** have a `MODE_COLLAPSE = 4` variant ;
///      collapsing it into `Converged` discarded a wire-stable
///      signal that downstream consumers (audit logs, dashboards)
///      relied on.
///   2. Mode-3 aggregation buckets sub-verdicts by their proto u8.
///      Mapping `ModeCollapse` to `2` made it indistinguishable
///      from a real `Converged` verdict — the aggregator would
///      produce a misleadingly-confident final verdict on swarms
///      that actually mode-collapsed.
///
/// The fix mirrors the canonical M4 server bridge
/// (`efe-server/src/bridge/verdict.rs::to_proto`) where
/// `Verdict::ModeCollapse` ⇒ `ProtoVerdict::ModeCollapse` (= 4).
#[must_use]
pub fn verdict_to_proto_u8(v: &crate::Verdict) -> u8 {
    match v {
        crate::Verdict::Continue => 1,
        crate::Verdict::Converged { .. } => 2,
        crate::Verdict::Divergence { .. } => 3,
        crate::Verdict::ModeCollapse => 4,
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
    fn verdict_to_proto_u8_known_values() {
        assert_eq!(verdict_to_proto_u8(&crate::Verdict::Continue), 1);
        assert_eq!(
            verdict_to_proto_u8(&crate::Verdict::Converged { confidence: 0.9 }),
            2
        );
        assert_eq!(
            verdict_to_proto_u8(&crate::Verdict::Divergence { reason: "x" }),
            3
        );
        // **Audit fix C-A3** — ModeCollapse maps to MODE_COLLAPSE (= 4),
        // NOT to Converged (= 2). The pre-fix mapping caused Mode-3
        // aggregation to bucket mode-collapsed swarms as Converged.
        assert_eq!(verdict_to_proto_u8(&crate::Verdict::ModeCollapse), 4);
    }

    #[test]
    fn verdict_to_proto_u8_distinct_per_variant() {
        // **Audit fix C-A3 anti-regression** — every Verdict variant
        // maps to a distinct u8. Pre-fix Continue=1, Converged=2,
        // Divergence=3, ModeCollapse=2 collided.
        let mapped: std::collections::HashSet<u8> = [
            verdict_to_proto_u8(&crate::Verdict::Continue),
            verdict_to_proto_u8(&crate::Verdict::Converged { confidence: 0.5 }),
            verdict_to_proto_u8(&crate::Verdict::Divergence { reason: "x" }),
            verdict_to_proto_u8(&crate::Verdict::ModeCollapse),
        ]
        .iter()
        .copied()
        .collect();
        assert_eq!(
            mapped.len(),
            4,
            "Verdict→u8 mapping must be 1-to-1 ; collision = silent data loss"
        );
    }

    #[test]
    fn synthesis_constants_exercise_simd_chunks() {
        // SYNTH_DIM = 8 must match the SIMD chunk size of CPU
        // kernels (AVX2 = 8 f32, NEON = 4 f32 × 2). Anti-regression :
        // if we ever change SYNTH_DIM, we must also bench the impact
        // on Mode-2/3 latency.
        assert_eq!(SYNTH_DIM, 8);
    }
}
