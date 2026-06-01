//! **Sprint D33** — Critique / diversification nano-agent template (MVP).
//!
//! ## Rôle
//!
//! NaCrit is the **anti-mode-collapse** safeguard. It explores the
//! latent space *away* from the dominant cluster (NaSem) and votes
//! against early consensus when alternative interpretations exist.
//!
//! Its centroid initialisation is `Uniform` (vs the default
//! `PerturbProblem`) so its agents start scattered across the latent
//! space. This is essential for SLO `D5-MOD-COL-1` (mode collapse
//! resilience).
//!
//! Default share 5 % — small but always present.

use super::{CentroidInit, FusionKernelId, NanoAgentTemplate};
use crate::types::TemplateCode;

/// Critique / diversification template.
///
/// MVP-era. Anchors the M5 fusion engine's "critic" votes — when the
/// other agents are converging on an obviously-wrong answer, NaCrit's
/// uniform-init agents act as a sanity check that pulls consensus
/// back toward the correct centroid.
#[derive(Debug, Clone, Copy, Default)]
pub struct NaCrit;

impl NanoAgentTemplate for NaCrit {
    fn code(&self) -> TemplateCode {
        TemplateCode::NaCrit
    }

    fn default_share(&self) -> f32 {
        // 5 % — always present, never dominant.
        0.05
    }

    fn fusion_kernel(&self) -> FusionKernelId {
        FusionKernelId::Diversifying
    }

    fn consensus_weight(&self) -> f32 {
        // Baseline 1.0 — NaCrit's vote counts the same as NaSem's
        // even though it explores differently.
        1.0
    }

    fn centroid_init(&self) -> Option<CentroidInit> {
        // **Sprint D33** — explicit Uniform init so NaCrit agents
        // start scattered. Pre-D33 this was hardcoded inside the
        // genesis spawner ; the trait override makes the contract
        // explicit + grep-able.
        Some(CentroidInit::Uniform)
    }

    fn description(&self) -> &'static str {
        "Critique / diversification template — anti-mode-collapse exploration"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn na_crit_has_correct_code() {
        assert_eq!(NaCrit.code(), TemplateCode::NaCrit);
        assert_eq!(NaCrit.code().as_u8(), 2);
        assert_eq!(NaCrit.code().as_str(), "na-crit");
    }

    #[test]
    fn na_crit_uses_uniform_init() {
        match NaCrit.centroid_init() {
            Some(CentroidInit::Uniform) => {}
            other => panic!("expected Uniform init, got {other:?}"),
        }
    }

    #[test]
    fn na_crit_kernel_is_diversifying() {
        assert_eq!(NaCrit.fusion_kernel(), FusionKernelId::Diversifying);
    }

    #[test]
    fn na_crit_is_mvp() {
        assert!(NaCrit.code().is_mvp());
    }
}
