//! **Sprint D33** — Temporal-aware nano-agent template (GA addition).
//!
//! ## Rôle
//!
//! NaTemp encodes **temporal constraints** : ordering (before/after),
//! durations (hours/days/years), recurrence (daily/weekly), timeline
//! reasoning. Pairs naturally with [`super::NaCausal`] for queries
//! that involve causal-temporal logic ("because of X yesterday,
//! Y will happen tomorrow").
//!
//! The fusion kernel maintains a "time-axis projection" of the
//! latent space — a learned linear transformation that compresses
//! temporal markers into a 1-D dimension. NaTemp agents project
//! their centroids through this transform before the standard
//! semantic kernel.
//!
//! Default share : 8 %. Activated more aggressively at genesis time
//! when the query contains temporal markers (regex pre-filter,
//! PII-free).
//!
//! Consensus weight 1.0 (baseline).

use super::{FusionKernelId, NanoAgentTemplate};
use crate::types::TemplateCode;

/// Temporal-aware template (D33 GA addition).
#[derive(Debug, Clone, Copy, Default)]
pub struct NaTemp;

impl NanoAgentTemplate for NaTemp {
    fn code(&self) -> TemplateCode {
        TemplateCode::NaTemp
    }

    fn default_share(&self) -> f32 {
        // 8 % — activated for time-aware queries.
        0.08
    }

    fn fusion_kernel(&self) -> FusionKernelId {
        FusionKernelId::Temporal
    }

    fn consensus_weight(&self) -> f32 {
        1.0
    }

    fn description(&self) -> &'static str {
        "Temporal-aware template — ordering, durations, recurrence, timeline"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn na_temp_has_correct_code() {
        assert_eq!(NaTemp.code(), TemplateCode::NaTemp);
        assert_eq!(NaTemp.code().as_u8(), 5);
        assert_eq!(NaTemp.code().as_str(), "na-temp");
    }

    #[test]
    fn na_temp_share_is_8_percent() {
        assert!((NaTemp.default_share() - 0.08).abs() < 1e-6);
    }

    #[test]
    fn na_temp_kernel_is_temporal() {
        assert_eq!(NaTemp.fusion_kernel(), FusionKernelId::Temporal);
    }

    #[test]
    fn na_temp_is_not_mvp() {
        assert!(!NaTemp.code().is_mvp());
    }
}
