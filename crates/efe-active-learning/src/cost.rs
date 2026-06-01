//! Cost governance for M24 active learning.
//!
//! Extends the M21 budget infrastructure with a new `active_learning`
//! category. Hard-cap pre-call estimate + daily reconciliation.

use crate::{ActiveLearningError, ActiveLearningResult, TenantId};
use sqlx::PgPool;

/// Approximate token count (whitespace + 1.3× heuristic for sub-word).
pub fn approximate_tokens(text: &str) -> u32 {
    let words = text.split_whitespace().count() as u32;
    ((words as f32) * 1.3).ceil() as u32
}

/// Estimate cost in cents for a labeling LLM call given the rendered
/// prompt and a model price card.
pub fn estimate_cost_cents(
    rendered_prompt: &str, output_tokens: u32,
    input_per_1k_cents: f32, output_per_1k_cents: f32,
) -> u32 {
    let input_tokens = approximate_tokens(rendered_prompt);
    let input_cost = (input_tokens as f32) * input_per_1k_cents / 1000.0;
    let output_cost = (output_tokens as f32) * output_per_1k_cents / 1000.0;
    (input_cost + output_cost).ceil() as u32
}

/// Check if the tenant has remaining budget for at least N more
/// labeling calls. Returns false if budget would be exceeded.
pub async fn has_budget_for_labeling(
    pg: &PgPool, tenant: &TenantId, n_calls: u32,
) -> ActiveLearningResult<bool> {
    // Read budget config + spent today.
    let row: Option<(i64, i64)> = sqlx::query_as(
        r#"
        SELECT
            COALESCE(tf.m24_active_learning_budget_cents_per_day, 0) AS budget,
            COALESCE(SUM(llc.cost_cents) FILTER (
                WHERE llc.budget_category = 'active_learning'
                  AND llc.created_at >= CURRENT_DATE
            ), 0)::BIGINT AS spent_today
        FROM tenant_feature tf
        LEFT JOIN llm_call_audit llc ON llc.tenant_id = tf.id
        WHERE tf.id = $1
        GROUP BY tf.m24_active_learning_budget_cents_per_day
        "#,
    )
    .bind(&tenant.0)
    .fetch_optional(pg).await?;

    let Some((budget, spent)) = row else {
        // Tenant feature row missing — fail closed (no budget).
        return Ok(false);
    };
    if budget <= 0 { return Ok(false); }  // 0 budget = never allowed
    let remaining = budget - spent;
    let estimate = (n_calls as i64) * 5;  // conservative ~5 cents per call
    Ok(remaining > estimate)
}

/// Mark a budget overrun event in the audit chain (called from worker
/// when batch is interrupted mid-flight by budget exhaustion).
pub async fn record_budget_exceeded(
    pg: &PgPool, tenant: &TenantId, spent: i64, budget: i64,
) -> ActiveLearningResult<()> {
    let mut tx = pg.begin().await?;
    crate::audit::append_learning_audit(
        &mut tx, tenant, None, None, None, None,
        "budget_exceeded", "system", None,
        serde_json::json!({
            "spent_cents": spent,
            "budget_cents": budget,
            "category": "active_learning",
            "timestamp": chrono::Utc::now(),
        }),
    ).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_approximation_basic() {
        assert!(approximate_tokens("hello world") > 1);
        assert!(approximate_tokens("the quick brown fox jumps over the lazy dog") > 8);
    }

    #[test]
    fn cost_estimate_zero_for_empty() {
        let c = estimate_cost_cents("", 0, 0.001, 0.005);
        assert_eq!(c, 0);
    }

    #[test]
    fn cost_estimate_realistic() {
        // A 500-token input + 200-token output at sonnet pricing
        let prompt = "x ".repeat(500);  // ~500 tokens approx
        let cost = estimate_cost_cents(&prompt, 200, 3.0, 15.0);
        assert!(cost > 0);
        assert!(cost < 1000); // sane upper bound
    }
}
