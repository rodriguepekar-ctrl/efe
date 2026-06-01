//! **Sprint D33** — Internal-consistency nano-agent template (GA addition).
//!
//! ## Rôle
//!
//! NaConsist detects **contradictions inside a single response** —
//! e.g. "X is true" + "X is false" appearing across different rounds
//! of the same swarm. Used by Mode-3 to discount sub-swarms whose
//! own internal centroids disagree across rounds (a proxy for
//! "this swarm is unstable").
//!
//! The fusion kernel computes a per-round consistency score by
//! comparing the current centroid against the running average of
//! prior round centroids. High variance ⇒ low consistency ⇒
//! NaConsist agents pull confidence DOWN ; low variance ⇒ high
//! consistency ⇒ they boost it.
//!
//! Default share : 7 %. Consensus weight 1.5 — consistent agents
//! boost the verdict ; inconsistent ones drag it down.

use super::{FusionKernelId, NanoAgentTemplate};
use crate::types::TemplateCode;

/// Internal-consistency template (D33 GA addition).
#[derive(Debug, Clone, Copy, Default)]
pub struct NaConsist;

impl NanoAgentTemplate for NaConsist {
    fn code(&self) -> TemplateCode {
        TemplateCode::NaConsist
    }

    fn default_share(&self) -> f32 {
        // 7 % — moderate share for cross-round stability checks.
        0.07
    }

    fn fusion_kernel(&self) -> FusionKernelId {
        FusionKernelId::Consistency
    }

    fn consensus_weight(&self) -> f32 {
        // 1.5 — consistency boosts confidence beyond baseline,
        // but less than safety (which is 2.0).
        1.5
    }

    fn description(&self) -> &'static str {
        "Internal-consistency template — cross-round contradiction detection"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn na_consist_has_correct_code() {
        assert_eq!(NaConsist.code(), TemplateCode::NaConsist);
        assert_eq!(NaConsist.code().as_u8(), 7);
        assert_eq!(NaConsist.code().as_str(), "na-consist");
    }

    #[test]
    fn na_consist_share_is_7_percent() {
        assert!((NaConsist.default_share() - 0.07).abs() < 1e-6);
    }

    #[test]
    fn na_consist_consensus_weight_is_1_5() {
        assert!((NaConsist.consensus_weight() - 1.5).abs() < 1e-6);
    }

    #[test]
    fn na_consist_weight_below_safety() {
        // Institutional invariant : consistency contributes but
        // doesn't outrank safety.
        use super::super::NaSafety;
        assert!(NaConsist.consensus_weight() < NaSafety.consensus_weight());
    }

    #[test]
    fn na_consist_kernel_is_consistency() {
        assert_eq!(NaConsist.fusion_kernel(), FusionKernelId::Consistency);
    }

    #[test]
    fn na_consist_is_not_mvp() {
        assert!(!NaConsist.code().is_mvp());
    }
}
