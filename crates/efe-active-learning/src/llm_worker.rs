//! LLM labeling worker — drains gold_label_queue, calls LLM via M21
//! router, parses + validates response, inserts gold_label_proposal.
//!
//! See § 9 of EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md.

use crate::{
    audit, cost, queue,
    ActiveLearningError, ActiveLearningResult, TenantId,
};
use crate::metrics::{
    AL_LABELS_PRODUCED_TOTAL, AL_LLM_WORKER_BUDGET_BLOCKED_TOTAL,
    AL_LLM_WORKER_ERRORS_TOTAL,
};
use crate::queue::QueueRow;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

const BATCH_SIZE_DEFAULT: u32 = 50;

/// Output schema of the `label_classify_v1` template.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LabelClassifyOutput {
    pub class_id: String,
    pub confidence: f32,
    pub rationale: String,
}

pub fn parse_label_output(json: &str) -> ActiveLearningResult<LabelClassifyOutput> {
    let parsed: LabelClassifyOutput = serde_json::from_str(json)
        .map_err(|e| ActiveLearningError::LlmSchemaInvalid(format!("JSON parse : {}", e)))?;
    if !efe_curation::is_valid_kebab_case(&parsed.class_id) {
        return Err(ActiveLearningError::LlmSchemaInvalid(
            format!("invalid class_id : '{}'", parsed.class_id)
        ));
    }
    if !(0.0..=1.0).contains(&parsed.confidence) {
        return Err(ActiveLearningError::LlmSchemaInvalid(
            format!("confidence out of range : {}", parsed.confidence)
        ));
    }
    if parsed.rationale.is_empty() || parsed.rationale.len() > 2048 {
        return Err(ActiveLearningError::LlmSchemaInvalid(
            "rationale empty or too long (>2048)".into()
        ));
    }
    Ok(parsed)
}

/// Trait : LLM client (mockable). Real impl wraps M21 LLMRouter.
#[async_trait::async_trait]
pub trait LlmClient: Send + Sync {
    async fn label(
        &self, tenant: &TenantId, prompt: &str, max_tokens: u32,
    ) -> ActiveLearningResult<LlmResponse>;
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
    pub provider: String,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_cents: u32,
    pub latency_ms: u32,
    pub audit_id: Option<Uuid>,
}

/// Trait : prompt renderer (uses efe-prompts at runtime).
#[async_trait::async_trait]
pub trait PromptRenderer: Send + Sync {
    async fn render(
        &self, tenant: &TenantId, queue_row: &QueueRow,
    ) -> ActiveLearningResult<RenderedPrompt>;
}

#[derive(Debug, Clone)]
pub struct RenderedPrompt {
    pub body: String,
    pub template_name: String,
    pub template_version: u32,
}

/// Trait : catalog class validator (so we can reject unknown classes
/// proposed by the LLM before inserting).
#[async_trait::async_trait]
pub trait ClassValidator: Send + Sync {
    async fn is_valid(
        &self, pg: &PgPool, tenant: &TenantId, catalog_id: &str, class_id: &str,
    ) -> ActiveLearningResult<bool>;
}

/// Process one queue row : render prompt → LLM call → parse → insert
/// proposal → transition queue → audit.
pub async fn label_one(
    pg: &PgPool, llm: &dyn LlmClient, renderer: &dyn PromptRenderer,
    validator: &dyn ClassValidator,
    tenant: &TenantId, row: &QueueRow,
) -> ActiveLearningResult<Uuid> {
    let prompt = renderer.render(tenant, row).await?;
    let resp = llm.label(tenant, &prompt.body, 200).await?;
    let parsed = parse_label_output(&resp.content)?;

    if !validator.is_valid(pg, tenant, &row.catalog_id, &parsed.class_id).await? {
        return Err(ActiveLearningError::LlmUnknownClass(parsed.class_id));
    }

    let proposal_id = Uuid::new_v4();
    let mut tx = pg.begin().await?;

    sqlx::query(
        r#"INSERT INTO gold_label_proposal
           (id, queue_id, tenant_id, proposed_class_id, llm_confidence,
            llm_rationale, llm_call_audit_id, prompt_template_name,
            prompt_template_version, auto_confirmed, created_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, FALSE, NOW())"#,
    )
    .bind(proposal_id).bind(row.id).bind(&tenant.0)
    .bind(&parsed.class_id).bind(parsed.confidence as f64)
    .bind(&parsed.rationale).bind(resp.audit_id)
    .bind(&prompt.template_name).bind(prompt.template_version as i32)
    .execute(&mut *tx).await?;

    sqlx::query(
        r#"UPDATE gold_label_queue
           SET status = 'LLM_LABELED'::gold_label_queue_status, labeled_at = NOW()
           WHERE id = $1"#,
    )
    .bind(row.id).execute(&mut *tx).await?;

    audit::append_learning_audit(
        &mut tx, tenant, Some(row.id), Some(proposal_id), None, None,
        "llm_label_proposed", "system", None,
        serde_json::json!({
            "queue_id": row.id, "proposed_class": parsed.class_id,
            "llm_confidence": parsed.confidence,
            "provider": resp.provider, "model": resp.model,
            "cost_cents": resp.cost_cents, "input_tokens": resp.input_tokens,
            "output_tokens": resp.output_tokens, "latency_ms": resp.latency_ms,
            "llm_call_audit_id": resp.audit_id,
        }),
    ).await?;

    tx.commit().await?;
    Ok(proposal_id)
}

/// Drain a batch for one tenant. Set RLS first ; check budget ; dequeue ;
/// label-each ; emit metrics.
pub async fn drain_one_tenant(
    pg: &PgPool, llm: &dyn LlmClient, renderer: &dyn PromptRenderer,
    validator: &dyn ClassValidator,
    tenant: &TenantId, batch_size: u32,
) -> ActiveLearningResult<u64> {
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(&tenant.0).execute(pg).await?;

    if !cost::has_budget_for_labeling(pg, tenant, batch_size).await? {
        AL_LLM_WORKER_BUDGET_BLOCKED_TOTAL
            .with_label_values(&[&tenant.0]).inc();
        return Ok(0);
    }

    let mut tx = pg.begin().await?;
    let batch = queue::dequeue_batch(&mut tx, tenant, batch_size).await?;
    if batch.is_empty() { tx.rollback().await?; return Ok(0); }
    tx.commit().await?;

    let mut produced = 0u64;
    for row in &batch {
        match label_one(pg, llm, renderer, validator, tenant, row).await {
            Ok(_proposal_id) => {
                produced += 1;
                AL_LABELS_PRODUCED_TOTAL
                    .with_label_values(&[&tenant.0, "success"]).inc();
            }
            Err(e) => {
                tracing::warn!(?e, queue_id=%row.id, "label_one failed");
                if let Err(e2) = queue::mark_failed(pg, row.id, &format!("{}", e)).await {
                    tracing::error!(?e2, "mark_failed errored");
                }
                AL_LABELS_PRODUCED_TOTAL
                    .with_label_values(&[&tenant.0, "fail"]).inc();
            }
        }
    }
    Ok(produced)
}

/// Long-running scheduled loop. Runs every `interval_secs` seconds.
pub async fn worker_loop(
    pg: Arc<PgPool>,
    llm: Arc<dyn LlmClient>,
    renderer: Arc<dyn PromptRenderer>,
    validator: Arc<dyn ClassValidator>,
    tenants: Arc<dyn crate::sampler::TenantRegistry>,
    interval_secs: u64,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
    loop {
        interval.tick().await;
        let snapshot = match tenants.iter_active_with_al().await {
            Ok(t) => t,
            Err(e) => { tracing::error!(?e, "fetch tenants failed"); continue; }
        };
        for tc in snapshot {
            match drain_one_tenant(
                &pg, llm.as_ref(), renderer.as_ref(), validator.as_ref(),
                &tc.id, BATCH_SIZE_DEFAULT,
            ).await {
                Ok(n) if n > 0 => tracing::info!(tenant=%tc.id.0, count=n, "labeled batch"),
                Ok(_) => {}
                Err(e) => {
                    AL_LLM_WORKER_ERRORS_TOTAL.with_label_values(&[&tc.id.0]).inc();
                    tracing::error!(?e, tenant=%tc.id.0, "drain failed");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_output() {
        let json = r#"{"class_id":"shipping-delay","confidence":0.85,"rationale":"package late"}"#;
        let out = parse_label_output(json).unwrap();
        assert_eq!(out.class_id, "shipping-delay");
        assert!((out.confidence - 0.85).abs() < 1e-6);
    }

    #[test]
    fn rejects_invalid_class_id() {
        let json = r#"{"class_id":"BadID","confidence":0.5,"rationale":"x"}"#;
        assert!(parse_label_output(json).is_err());
    }

    #[test]
    fn rejects_oob_confidence() {
        let json = r#"{"class_id":"a-b","confidence":1.5,"rationale":"x"}"#;
        assert!(parse_label_output(json).is_err());
    }

    #[test]
    fn rejects_empty_rationale() {
        let json = r#"{"class_id":"a-b","confidence":0.5,"rationale":""}"#;
        assert!(parse_label_output(json).is_err());
    }

    #[test]
    fn rejects_overlong_rationale() {
        let big = "x".repeat(3000);
        let json = format!(r#"{{"class_id":"a-b","confidence":0.5,"rationale":"{}"}}"#, big);
        assert!(parse_label_output(&json).is_err());
    }
}
