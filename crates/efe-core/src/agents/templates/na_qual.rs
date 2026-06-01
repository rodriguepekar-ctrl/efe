//! **Sprint D33** — Quality-focused nano-agent template (GA addition).
//!
//! ## Rôle
//!
//! NaQual scores **output quality** along three axes :
//!   1. **Coherence** — no contradictions internal to a span.
//!   2. **Fluency** — low cross-entropy on the dissolution decoder.
//!   3. **Pertinence** — high cosine similarity to `problem_embedding`.
//!
//! The fusion kernel applies a learned quality-prior to nudge agents
//! away from low-quality regions of the latent space. NaQual is a
//! *scoring* agent, not a generative one — it doesn't propose a
//! verdict, it grades the proposals from other templates.
//!
//! Default share : 15 % (second-highest after NaSem's 50 %).
//! Consensus weight 1.2 — slightly higher than baseline because
//! quality is a meta-signal that should boost confident high-quality
//! verdicts and discount low-quality ones.

use super::{FusionKernelId, NanoAgentTemplate};
use crate::types::TemplateCode;

/// Quality-focused template (D33 GA addition).
#[derive(Debug, Clone, Copy, Default)]
pub struct NaQual;

impl NanoAgentTemplate for NaQual {
    fn code(&self) -> TemplateCode {
        TemplateCode::NaQual
    }

    fn default_share(&self) -> f32 {
        // 15 % — second-highest share after NaSem (50 %).
        0.15
    }

    fn fusion_kernel(&self) -> FusionKernelId {
        FusionKernelId::Quality
    }

    fn consensus_weight(&self) -> f32 {
        // 1.2 — quality boosts confidence beyond baseline.
        1.2
    }

    fn description(&self) -> &'static str {
        "Quality-focused template — coherence + fluency + pertinence scoring"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn na_qual_has_correct_code() {
        assert_eq!(NaQual.code(), TemplateCode::NaQual);
        assert_eq!(NaQual.code().as_u8(), 3);
        assert_eq!(NaQual.code().as_str(), "na-qual");
    }

    #[test]
    fn na_qual_share_is_15_percent() {
        assert_eq!(NaQual.default_share(), 0.15);
    }

    #[test]
    fn na_qual_kernel_is_quality() {
        assert_eq!(NaQual.fusion_kernel(), FusionKernelId::Quality);
    }

    #[test]
    fn na_qual_consensus_weight_above_baseline() {
        assert!(NaQual.consensus_weight() > 1.0);
        assert_eq!(NaQual.consensus_weight(), 1.2);
    }

    #[test]
    fn na_qual_is_not_mvp() {
        assert!(!NaQual.code().is_mvp());
    }
}
