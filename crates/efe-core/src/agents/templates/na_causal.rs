//! **Sprint D33** — Causal-reasoning nano-agent template (GA addition).
//!
//! ## Rôle
//!
//! NaCausal specialises in **directed causal reasoning** — detecting
//! and scoring assertions of the form "X causes Y" vs spurious
//! correlations. Used heavily by Mode-3 sub-swarms when the input
//! query contains causal markers : "because", "if X then Y",
//! counterfactuals ("what if X had been Y").
//!
//! The fusion kernel evaluates a per-pair (X, Y) causal score against
//! a learned prior derived from the input embedding. Nano-agents
//! converge toward configurations that maximise the causal signal.
//!
//! Default share : 10 %. Consensus weight 1.0 (baseline) — causal
//! reasoning is informative but doesn't outrank quality or safety.

use super::{FusionKernelId, NanoAgentTemplate};
use crate::types::TemplateCode;

/// Causal-reasoning template (D33 GA addition).
#[derive(Debug, Clone, Copy, Default)]
pub struct NaCausal;

impl NanoAgentTemplate for NaCausal {
    fn code(&self) -> TemplateCode {
        TemplateCode::NaCausal
    }

    fn default_share(&self) -> f32 {
        // 10 % — moderate share for queries with causal markers.
        0.10
    }

    fn fusion_kernel(&self) -> FusionKernelId {
        FusionKernelId::Causal
    }

    fn consensus_weight(&self) -> f32 {
        1.0
    }

    fn description(&self) -> &'static str {
        "Causal-reasoning template — directed X→Y inference and counterfactuals"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn na_causal_has_correct_code() {
        assert_eq!(NaCausal.code(), TemplateCode::NaCausal);
        assert_eq!(NaCausal.code().as_u8(), 4);
        assert_eq!(NaCausal.code().as_str(), "na-causal");
    }

    #[test]
    fn na_causal_share_is_10_percent() {
        assert_eq!(NaCausal.default_share(), 0.10);
    }

    #[test]
    fn na_causal_kernel_is_causal() {
        assert_eq!(NaCausal.fusion_kernel(), FusionKernelId::Causal);
    }

    #[test]
    fn na_causal_is_not_mvp() {
        assert!(!NaCausal.code().is_mvp());
    }
}
