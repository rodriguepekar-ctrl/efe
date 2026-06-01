//! Auto-confirm gate — 3 cumulative conditions, all required.
//!
//! See § 11 of EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md.

use crate::{
    ActiveLearningError, ActiveLearningResult, StakesTier, TenantActiveLearningFlags, TenantId,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// Result of the gate check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoConfirmDecision {
    Allowed { reasons: Vec<String> },
    Blocked { reasons: Vec<String> },
}

impl AutoConfirmDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }
}

#[derive(Debug, Clone)]
pub struct GateInputs {
    pub llm_confidence: f32,
    pub auto_confirm_threshold: f64,
    pub dpo_consent_active: bool,
    pub stakes_tier: StakesTier,
}

/// Pure decision function — no DB access. The 3 conditions :
///   1. llm_confidence >= tenant.auto_confirm_threshold
///   2. tenant.dpo_consent_active = true
///   3. catalog.stakes_tier == LOW
pub fn decide(inputs: &GateInputs) -> AutoConfirmDecision {
    let mut reasons_blocked = Vec::new();
    let mut reasons_allowed = Vec::new();

    if (inputs.llm_confidence as f64) >= inputs.auto_confirm_threshold {
        reasons_allowed.push(format!(
            "confidence {} ≥ threshold {}", inputs.llm_confidence, inputs.auto_confirm_threshold
        ));
    } else {
        reasons_blocked.push(format!(
            "confidence {} < threshold {}", inputs.llm_confidence, inputs.auto_confirm_threshold
        ));
    }

    if inputs.dpo_consent_active {
        reasons_allowed.push("DPO consent active".into());
    } else {
        reasons_blocked.push("DPO consent NOT active".into());
    }

    if inputs.stakes_tier == StakesTier::Low {
        reasons_allowed.push("catalog stakes_tier = LOW".into());
    } else {
        reasons_blocked.push(format!(
            "catalog stakes_tier = {} (must be LOW)", inputs.stakes_tier.as_str()
        ));
    }

    if reasons_blocked.is_empty() {
        AutoConfirmDecision::Allowed { reasons: reasons_allowed }
    } else {
        AutoConfirmDecision::Blocked { reasons: reasons_blocked }
    }
}

/// Resolve gate inputs from DB and decide.
pub async fn check_proposal(
    pg: &PgPool, tenant: &TenantId, proposal_id: Uuid,
    flags: &TenantActiveLearningFlags,
) -> ActiveLearningResult<AutoConfirmDecision> {
    let row: Option<(f64, String, StakesTier)> = sqlx::query_as(
        r#"SELECT
              p.llm_confidence,
              glq.catalog_id,
              c.m24_stakes_tier
           FROM gold_label_proposal p
           JOIN gold_label_queue glq ON glq.id = p.queue_id
           JOIN catalog c ON c.id = glq.catalog_id AND c.tenant_id = p.tenant_id
           WHERE p.id = $1 AND p.tenant_id = $2"#,
    )
    .bind(proposal_id).bind(&tenant.0)
    .fetch_optional(pg).await?;
    let (llm_conf, _catalog_id, tier) = row
        .ok_or(ActiveLearningError::ProposalNotFound(proposal_id))?;

    Ok(decide(&GateInputs {
        llm_confidence: llm_conf as f32,
        auto_confirm_threshold: flags.auto_confirm_threshold,
        dpo_consent_active: flags.dpo_consent_active,
        stakes_tier: tier,
    }))
}

/// Apply auto-confirm if gate allows : insert gold_example, transition
/// queue, audit. Returns true if applied.
pub async fn apply_if_allowed(
    pg: &PgPool, tenant: &TenantId, proposal_id: Uuid,
    flags: &TenantActiveLearningFlags,
) -> ActiveLearningResult<bool> {
    let decision = check_proposal(pg, tenant, proposal_id, flags).await?;
    let allowed = decision.is_allowed();
    crate::metrics::AL_AUTO_CONFIRM_TOTAL
        .with_label_values(&[&tenant.0, if allowed { "allowed" } else { "blocked" }])
        .inc();
    if !allowed { return Ok(false); }

    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(&tenant.0).execute(pg).await?;

    let row: Option<(Uuid, String, String, String)> = sqlx::query_as(
        r#"SELECT glq.id, glq.catalog_id, glq.sentence_redacted, p.proposed_class_id
           FROM gold_label_proposal p
           JOIN gold_label_queue glq ON glq.id = p.queue_id
           WHERE p.id = $1 AND p.tenant_id = $2"#,
    )
    .bind(proposal_id).bind(&tenant.0)
    .fetch_optional(pg).await?;
    let (queue_id, catalog_id, sentence, class_id) = row
        .ok_or(ActiveLearningError::ProposalNotFound(proposal_id))?;

    let mut tx = pg.begin().await?;

    // Insert gold_example
    let dataset_id: Uuid = sqlx::query_scalar(
        r#"SELECT id FROM gold_dataset
           WHERE tenant_id = $1 AND catalog_id = $2
           ORDER BY created_at DESC LIMIT 1"#,
    )
    .bind(&tenant.0).bind(&catalog_id)
    .fetch_optional(&mut *tx).await?
    .ok_or_else(|| ActiveLearningError::InvalidInput(
        format!("no gold_dataset for tenant={} catalog={}", tenant.0, catalog_id)
    ))?;
    let example_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO gold_example
           (id, gold_dataset_id, text, expected_class_id, source, created_at)
           VALUES ($1, $2, $3, $4, 'production_overflow', NOW())"#,
    )
    .bind(example_id).bind(dataset_id).bind(&sentence).bind(&class_id)
    .execute(&mut *tx).await?;

    // Mark proposal auto_confirmed
    sqlx::query(
        "UPDATE gold_label_proposal SET auto_confirmed = TRUE WHERE id = $1"
    ).bind(proposal_id).execute(&mut *tx).await?;

    // Transition queue
    sqlx::query(
        r#"UPDATE gold_label_queue
           SET status = 'CONFIRMED'::gold_label_queue_status, confirmed_at = NOW()
           WHERE id = $1"#,
    )
    .bind(queue_id).execute(&mut *tx).await?;

    // Audit
    crate::audit::append_learning_audit(
        &mut tx, tenant, Some(queue_id), Some(proposal_id), None, None,
        "auto_confirmed", "system", None,
        serde_json::json!({
            "proposal_id": proposal_id,
            "gold_example_id": example_id,
            "decision": decision,
            "applied_at": Utc::now(),
        }),
    ).await?;

    tx.commit().await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(conf: f32, dpo: bool, tier: StakesTier) -> GateInputs {
        GateInputs {
            llm_confidence: conf,
            auto_confirm_threshold: 0.90,
            dpo_consent_active: dpo,
            stakes_tier: tier,
        }
    }

    #[test]
    fn all_3_conditions_allow() {
        let d = decide(&inputs(0.95, true, StakesTier::Low));
        assert!(d.is_allowed());
    }

    #[test]
    fn low_confidence_blocks() {
        let d = decide(&inputs(0.85, true, StakesTier::Low));
        assert!(!d.is_allowed());
    }

    #[test]
    fn no_dpo_blocks() {
        let d = decide(&inputs(0.99, false, StakesTier::Low));
        assert!(!d.is_allowed());
    }

    #[test]
    fn medium_stakes_blocks() {
        let d = decide(&inputs(0.99, true, StakesTier::Medium));
        assert!(!d.is_allowed());
    }

    #[test]
    fn high_stakes_blocks_even_with_perfect_conf() {
        let d = decide(&inputs(1.0, true, StakesTier::High));
        assert!(!d.is_allowed());
    }

    #[test]
    fn block_lists_all_failing_reasons() {
        let d = decide(&inputs(0.5, false, StakesTier::High));
        match d {
            AutoConfirmDecision::Blocked { reasons } => {
                assert_eq!(reasons.len(), 3);
            }
            _ => panic!("expected Blocked"),
        }
    }
}
