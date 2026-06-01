//! **Sprint D33** — Semantic-density nano-agent template (MVP).
//!
//! ## Refactor note
//!
//! Pre-D33 the NaSem behaviour lived implicitly inside
//! `genesis/registry.rs::StubTemplateRegistry`. The D33 refactor
//! extracts the descriptor into this dedicated module so :
//!
//!   1. The behavioural surface is symmetric with the GA additions
//!      (NaQual, NaCausal, …).
//!   2. Future tweaks to NaSem don't pollute the genesis registry.
//!   3. Tests can import `NaSem` directly without going through
//!      the spawner.
//!
//! ## Rôle
//!
//! NaSem is the **default-dominant** template. It carries 50 % of
//! the population in a balanced GA mix (highest share). Its kernel
//! optimises for semantic density — covering the input embedding's
//! local neighbourhood with a dense cluster of agents that vote
//! on the central tendency.
//!
//! Consensus weight = 1.0 (baseline) — NaSem is the reference voter.

use super::{FusionKernelId, NanoAgentTemplate};
use crate::types::TemplateCode;

/// Semantic-density template.
///
/// MVP-era default. Its share dominates a balanced GA mix because
/// most production queries are semantic-similarity tasks (lookup,
/// classification, RAG-style retrieval).
#[derive(Debug, Clone, Copy, Default)]
pub struct NaSem;

impl NanoAgentTemplate for NaSem {
    fn code(&self) -> TemplateCode {
        TemplateCode::NaSem
    }

    fn default_share(&self) -> f32 {
        // **45 %** — dominant in a balanced GA mix, sized so the
        // 8-template default_share sum is exactly 1.0 :
        //   NaSem 0.45 + NaLog 0.05 + NaCrit 0.05 +
        //   NaQual 0.15 + NaCausal 0.10 + NaTemp 0.08 +
        //   NaSafety 0.05 + NaConsist 0.07 = 1.00 ✓
        // The pin test `templates::tests::ga_default_shares_sum_to_unity`
        // enforces this invariant.
        0.45
    }

    fn fusion_kernel(&self) -> FusionKernelId {
        FusionKernelId::Semantic
    }

    fn consensus_weight(&self) -> f32 {
        // Baseline 1.0. NaSem is the reference voter.
        1.0
    }

    fn description(&self) -> &'static str {
        "Semantic-density template — central tendency / dense neighbourhood vote"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn na_sem_has_correct_code() {
        assert_eq!(NaSem.code(), TemplateCode::NaSem);
        assert_eq!(NaSem.code().as_u8(), 0);
        assert_eq!(NaSem.code().as_str(), "na-sem");
    }

    #[test]
    fn na_sem_share_is_dominant() {
        assert_eq!(NaSem.default_share(), 0.45);
    }

    #[test]
    fn na_sem_kernel_is_semantic() {
        assert_eq!(NaSem.fusion_kernel(), FusionKernelId::Semantic);
    }

    #[test]
    fn na_sem_consensus_weight_is_baseline() {
        assert_eq!(NaSem.consensus_weight(), 1.0);
    }

    #[test]
    fn na_sem_centroid_init_uses_default() {
        assert!(NaSem.centroid_init().is_none());
    }

    #[test]
    fn na_sem_is_mvp() {
        assert!(NaSem.code().is_mvp());
    }
}
