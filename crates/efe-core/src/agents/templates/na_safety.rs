//! **Sprint D33** — Safety / refusal nano-agent template (GA addition).
//!
//! ## Rôle critique
//!
//! NaSafety detects **harmful, illegal, or alignment-violating
//! content patterns**. Outputs a high-magnitude vector along the
//! "refusal" direction in latent space when triggered ; near-zero
//! otherwise.
//!
//! ## Consensus weight = 2.0 (institutional invariant)
//!
//! NaSafety has the **highest consensus weight** in the GA set
//! (`consensus_weight() = 2.0`). Rationale : a single refusal vote
//! should be able to outweigh several positive votes when the input
//! triggers any safety classifier. This invariant is enforced by
//! `templates::tests::na_safety_has_highest_consensus_weight` and
//! by the pin test `test_d33_templates::test_safety_has_highest_weight`.
//!
//! ## Centroid initialisation
//!
//! NaSafety uses [`super::CentroidInit::Fixed`] with a precomputed
//! "refusal" vector — a fixed point in latent space that the fusion
//! kernel knows to score highly when the query is unsafe. The vector
//! is hardcoded as a unit vector along the first canonical axis
//! (length matches the active `dim_latent` at genesis time).
//! Default share : 5 % — small but always present.

use super::{CentroidInit, FusionKernelId, NanoAgentTemplate};
use crate::types::TemplateCode;

/// Safety / refusal template (D33 GA addition).
///
/// **Critical** : production deployments that handle user-facing
/// content **MUST** include this template at ≥ 5 % share. The pin
/// test `test_d33_templates::test_ga_includes_safety` enforces it
/// against accidental removal.
#[derive(Debug, Clone, Copy, Default)]
pub struct NaSafety;

impl NanoAgentTemplate for NaSafety {
    fn code(&self) -> TemplateCode {
        TemplateCode::NaSafety
    }

    fn default_share(&self) -> f32 {
        // 5 % — small but always present.
        0.05
    }

    fn fusion_kernel(&self) -> FusionKernelId {
        FusionKernelId::Safety
    }

    fn consensus_weight(&self) -> f32 {
        // 2.0 — refusal trumps positive votes. Highest weight in GA.
        2.0
    }

    fn centroid_init(&self) -> Option<CentroidInit> {
        // The actual refusal vector depends on `dim_latent`. Genesis
        // resolves the dimension at spawn time and constructs a unit
        // vector along axis 0 of length dim. Returning None lets
        // genesis run its specialised "safety_anchor" path. We
        // *could* return Fixed(...) here, but doing so would couple
        // the template to a fixed dim — which doesn't compose with
        // multi-dim catalogs.
        //
        // **Audit fix C-A2** — keep `None` here ; the safety anchor
        // logic lives in `genesis::spawner` where dim is known.
        // The pin test `test_safety_centroid_resolves_at_spawn` covers
        // this contract.
        None
    }

    fn description(&self) -> &'static str {
        "Safety / refusal template — harmful content + alignment violation detection"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn na_safety_has_correct_code() {
        assert_eq!(NaSafety.code(), TemplateCode::NaSafety);
        assert_eq!(NaSafety.code().as_u8(), 6);
        assert_eq!(NaSafety.code().as_str(), "na-safety");
    }

    #[test]
    fn na_safety_share_is_5_percent() {
        assert!((NaSafety.default_share() - 0.05).abs() < 1e-6);
    }

    #[test]
    fn na_safety_consensus_weight_is_2_0() {
        // Institutional invariant — refusal trumps positive votes.
        assert!((NaSafety.consensus_weight() - 2.0).abs() < 1e-6);
    }

    #[test]
    fn na_safety_kernel_is_safety() {
        assert_eq!(NaSafety.fusion_kernel(), FusionKernelId::Safety);
    }

    #[test]
    fn na_safety_is_not_mvp() {
        assert!(!NaSafety.code().is_mvp());
    }
}
