//! Promotion gate — decides whether a retuned bundle can be auto-promoted
//! or must be flagged for operator approval.
//!
//! See § 16 of EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md.

use crate::{StakesTier, TenantActiveLearningFlags};
use crate::metrics::{AL_PROMOTIONS_BLOCKED_TOTAL, AL_PROMOTIONS_TOTAL};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromotionEligibility {
    AutoPromote { reasons: Vec<String> },
    FlagForOperator { reason: String },
    NotEligible { reason: String },
}

#[derive(Debug, Clone)]
pub struct GateInputs {
    pub flagged_for_promotion: bool,
    pub auto_promotion_enabled: bool,
    pub human_review_required: bool,
    pub dpo_consent_active: bool,
    pub catalog_stakes_tier: StakesTier,
}

/// Pure decision function. Returns one of three eligibility states.
pub fn evaluate(inputs: &GateInputs) -> PromotionEligibility {
    if !inputs.flagged_for_promotion {
        return PromotionEligibility::NotEligible {
            reason: "eval did not pass +1pp threshold".into(),
        };
    }

    if !inputs.auto_promotion_enabled {
        return PromotionEligibility::FlagForOperator {
            reason: "tenant.auto_promotion_enabled = false (operator approval required)".into(),
        };
    }

    if inputs.human_review_required {
        return PromotionEligibility::FlagForOperator {
            reason: "tenant.human_review_required = true".into(),
        };
    }

    if inputs.catalog_stakes_tier != StakesTier::Low {
        return PromotionEligibility::FlagForOperator {
            reason: format!(
                "catalog stakes_tier = {} (auto-promotion only allowed for LOW)",
                inputs.catalog_stakes_tier.as_str()
            ),
        };
    }

    if !inputs.dpo_consent_active {
        return PromotionEligibility::FlagForOperator {
            reason: "DPO consent NOT active".into(),
        };
    }

    PromotionEligibility::AutoPromote {
        reasons: vec![
            "flagged_for_promotion=true".into(),
            "auto_promotion_enabled=true".into(),
            "human_review_required=false".into(),
            format!("stakes_tier={}", inputs.catalog_stakes_tier.as_str()),
            "dpo_consent_active=true".into(),
        ],
    }
}

/// Convenience : record metric per outcome.
pub fn record_outcome(tenant_id: &str, eligibility: &PromotionEligibility) {
    match eligibility {
        PromotionEligibility::AutoPromote { .. } => {
            AL_PROMOTIONS_TOTAL.with_label_values(&[tenant_id, "auto"]).inc();
        }
        PromotionEligibility::FlagForOperator { reason } => {
            AL_PROMOTIONS_BLOCKED_TOTAL
                .with_label_values(&[tenant_id, classify_reason(reason)])
                .inc();
        }
        PromotionEligibility::NotEligible { reason } => {
            AL_PROMOTIONS_BLOCKED_TOTAL
                .with_label_values(&[tenant_id, classify_reason(reason)])
                .inc();
        }
    }
}

fn classify_reason(r: &str) -> &'static str {
    if r.contains("auto_promotion_enabled") { "auto_promotion_off" }
    else if r.contains("human_review_required") { "human_review_required" }
    else if r.contains("stakes_tier") { "stakes_tier" }
    else if r.contains("DPO consent") { "no_dpo_consent" }
    else if r.contains("eval did not pass") { "eval_failed" }
    else { "other" }
}

/// Resolve flags from tenant + catalog and decide.
pub fn evaluate_for_run(
    flagged_for_promotion: bool,
    flags: &TenantActiveLearningFlags,
    catalog_stakes_tier: StakesTier,
) -> PromotionEligibility {
    evaluate(&GateInputs {
        flagged_for_promotion,
        auto_promotion_enabled: flags.auto_promotion_enabled,
        human_review_required: flags.human_review_required,
        dpo_consent_active: flags.dpo_consent_active,
        catalog_stakes_tier,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(
        flagged: bool, auto: bool, hr_required: bool, dpo: bool, tier: StakesTier,
    ) -> GateInputs {
        GateInputs {
            flagged_for_promotion: flagged,
            auto_promotion_enabled: auto,
            human_review_required: hr_required,
            dpo_consent_active: dpo,
            catalog_stakes_tier: tier,
        }
    }

    #[test]
    fn not_flagged_not_eligible() {
        let r = evaluate(&inputs(false, true, false, true, StakesTier::Low));
        assert!(matches!(r, PromotionEligibility::NotEligible { .. }));
    }

    #[test]
    fn auto_promotion_disabled_flags_operator() {
        let r = evaluate(&inputs(true, false, false, true, StakesTier::Low));
        assert!(matches!(r, PromotionEligibility::FlagForOperator { .. }));
    }

    #[test]
    fn human_review_required_flags_operator() {
        let r = evaluate(&inputs(true, true, true, true, StakesTier::Low));
        assert!(matches!(r, PromotionEligibility::FlagForOperator { .. }));
    }

    #[test]
    fn medium_stakes_flags_operator() {
        let r = evaluate(&inputs(true, true, false, true, StakesTier::Medium));
        assert!(matches!(r, PromotionEligibility::FlagForOperator { .. }));
    }

    #[test]
    fn high_stakes_flags_operator() {
        let r = evaluate(&inputs(true, true, false, true, StakesTier::High));
        assert!(matches!(r, PromotionEligibility::FlagForOperator { .. }));
    }

    #[test]
    fn no_dpo_consent_flags_operator() {
        let r = evaluate(&inputs(true, true, false, false, StakesTier::Low));
        assert!(matches!(r, PromotionEligibility::FlagForOperator { .. }));
    }

    #[test]
    fn all_4_conditions_low_stakes_auto_promote() {
        let r = evaluate(&inputs(true, true, false, true, StakesTier::Low));
        assert!(matches!(r, PromotionEligibility::AutoPromote { .. }));
    }
}
