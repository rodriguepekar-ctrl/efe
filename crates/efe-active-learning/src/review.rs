//! Human review of LLM-proposed gold labels.
//!
//! Mirrors the M23 curation_review pattern, with three decisions :
//! CONFIRM, EDIT_AND_CONFIRM (operator changes class_id), REJECT.
//!
//! See § 10 of EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md.

use crate::{ActiveLearningError, ActiveLearningResult, TenantId};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

const OIDC_FRESHNESS_SECONDS: i64 = 600;  // 10 min
const MIN_RATIONALE_LEN: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "gold_label_review_decision", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewDecision {
    Confirm,
    EditAndConfirm,
    Reject,
}

impl ReviewDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Confirm => "CONFIRM",
            Self::EditAndConfirm => "EDIT_AND_CONFIRM",
            Self::Reject => "REJECT",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReviewRequest {
    pub proposal_id: Uuid,
    pub reviewer_oidc_sub: String,
    pub reviewer_role: String,
    pub decision: ReviewDecision,
    pub final_class_id: Option<String>,  // required for EditAndConfirm
    pub rationale: String,
    pub quality_tag: Option<String>,
    pub oidc_id_token_hash: String,
    pub oidc_token_iat: DateTime<Utc>,
}

/// Submit a review. Validates OIDC freshness, role, rationale length,
/// inserts gold_label_review row, updates queue status, optionally
/// inserts gold_example, appends audit.
pub async fn submit_review(
    pg: &PgPool, tenant: &TenantId, req: ReviewRequest,
) -> ActiveLearningResult<Uuid> {
    // 1. Validate input
    if req.rationale.trim().len() < MIN_RATIONALE_LEN {
        return Err(ActiveLearningError::InvalidInput(
            format!("rationale must be ≥ {} chars (Art. 14 audit)", MIN_RATIONALE_LEN)
        ));
    }
    let age = (Utc::now() - req.oidc_token_iat).num_seconds();
    if age > OIDC_FRESHNESS_SECONDS {
        return Err(ActiveLearningError::OidcTokenStale {
            age_seconds: age, max: OIDC_FRESHNESS_SECONDS,
        });
    }
    if matches!(req.decision, ReviewDecision::EditAndConfirm) && req.final_class_id.is_none() {
        return Err(ActiveLearningError::InvalidInput(
            "EDIT_AND_CONFIRM requires final_class_id".into()
        ));
    }

    // 2. Set RLS context
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(&tenant.0).execute(pg).await?;

    let mut tx = pg.begin().await?;

    // 3. Lookup proposal + queue + catalog
    let proposal: Option<(Uuid, String, String, String, String)> = sqlx::query_as(
        r#"SELECT p.id, p.tenant_id, glq.catalog_id, p.proposed_class_id, glq.sentence_redacted
           FROM gold_label_proposal p
           JOIN gold_label_queue glq ON glq.id = p.queue_id
           WHERE p.id = $1"#,
    )
    .bind(req.proposal_id)
    .fetch_optional(&mut *tx).await?;
    let (proposal_id, proposal_tenant, catalog_id, proposed_class, sentence) =
        proposal.ok_or(ActiveLearningError::ProposalNotFound(req.proposal_id))?;
    if proposal_tenant != tenant.0 {
        return Err(ActiveLearningError::TenantMismatch {
            caller: tenant.0.clone(), requested: proposal_tenant,
        });
    }

    // 4. Insert review row
    let review_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO gold_label_review
           (id, proposal_id, reviewer_oidc_sub, reviewer_role, decision,
            final_class_id, rationale, quality_tag,
            oidc_id_token_hash, oidc_token_iat, reviewed_at)
           VALUES ($1, $2, $3, $4, $5::gold_label_review_decision, $6, $7, $8, $9, $10, NOW())"#,
    )
    .bind(review_id).bind(proposal_id)
    .bind(&req.reviewer_oidc_sub).bind(&req.reviewer_role)
    .bind(req.decision.as_str())
    .bind(req.final_class_id.as_deref())
    .bind(&req.rationale).bind(req.quality_tag.as_deref())
    .bind(&req.oidc_id_token_hash).bind(req.oidc_token_iat)
    .execute(&mut *tx).await?;

    // 5. On confirm : insert gold_example + transition queue
    let queue_id: Uuid = sqlx::query_scalar(
        "SELECT queue_id FROM gold_label_proposal WHERE id = $1"
    ).bind(proposal_id).fetch_one(&mut *tx).await?;

    match req.decision {
        ReviewDecision::Confirm => {
            insert_gold_example(
                &mut tx, tenant, &catalog_id, &sentence, &proposed_class, "human_review_confirm",
            ).await?;
            transition_queue(&mut tx, queue_id, "CONFIRMED").await?;
            audit_event(&mut tx, tenant, queue_id, proposal_id, "human_review_confirm",
                        &req.reviewer_oidc_sub, &req.reviewer_role, &req.rationale).await?;
        }
        ReviewDecision::EditAndConfirm => {
            let final_class = req.final_class_id.as_ref().unwrap();
            insert_gold_example(
                &mut tx, tenant, &catalog_id, &sentence, final_class, "human_review_edit",
            ).await?;
            transition_queue(&mut tx, queue_id, "CONFIRMED").await?;
            audit_event(&mut tx, tenant, queue_id, proposal_id, "human_review_edit",
                        &req.reviewer_oidc_sub, &req.reviewer_role,
                        &format!("{} (edited: {} → {})", req.rationale, proposed_class, final_class)).await?;
        }
        ReviewDecision::Reject => {
            transition_queue(&mut tx, queue_id, "REJECTED").await?;
            audit_event(&mut tx, tenant, queue_id, proposal_id, "human_review_reject",
                        &req.reviewer_oidc_sub, &req.reviewer_role, &req.rationale).await?;
        }
    }

    crate::metrics::AL_REVIEWS_TOTAL
        .with_label_values(&[&tenant.0, req.decision.as_str()]).inc();

    tx.commit().await?;
    Ok(review_id)
}

async fn insert_gold_example(
    tx: &mut Transaction<'_, Postgres>,
    tenant: &TenantId, catalog_id: &str,
    sentence: &str, class_id: &str, source: &str,
) -> ActiveLearningResult<Uuid> {
    let dataset_id: Uuid = sqlx::query_scalar(
        r#"SELECT id FROM gold_dataset
           WHERE tenant_id = $1 AND catalog_id = $2
           ORDER BY created_at DESC
           LIMIT 1"#,
    )
    .bind(&tenant.0).bind(catalog_id)
    .fetch_optional(&mut **tx).await?
    .ok_or_else(|| ActiveLearningError::InvalidInput(
        format!("no gold_dataset for tenant={} catalog={}", tenant.0, catalog_id)
    ))?;
    let id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO gold_example
           (id, gold_dataset_id, text, expected_class_id, source, created_at)
           VALUES ($1, $2, $3, $4, $5, NOW())"#,
    )
    .bind(id).bind(dataset_id).bind(sentence).bind(class_id)
    .bind(map_review_source_to_gold(source))
    .execute(&mut **tx).await?;
    Ok(id)
}

fn map_review_source_to_gold(s: &str) -> &'static str {
    // gold_example.source enum (from M23 schema) accepts:
    // 'human_curated', 'llm_generated', 'production_overflow', 'imported'
    match s {
        "human_review_confirm" | "human_review_edit" => "human_curated",
        "auto_confirmed" => "production_overflow",
        _ => "imported",
    }
}

async fn transition_queue(
    tx: &mut Transaction<'_, Postgres>, id: Uuid, new_status: &str,
) -> ActiveLearningResult<()> {
    sqlx::query(
        r#"UPDATE gold_label_queue
           SET status = $1::gold_label_queue_status, confirmed_at = NOW()
           WHERE id = $2"#,
    )
    .bind(new_status).bind(id)
    .execute(&mut **tx).await?;
    Ok(())
}

async fn audit_event(
    tx: &mut Transaction<'_, Postgres>,
    tenant: &TenantId, queue_id: Uuid, proposal_id: Uuid,
    event_type: &str, actor: &str, role: &str, rationale: &str,
) -> ActiveLearningResult<()> {
    crate::audit::append_learning_audit(
        tx, tenant, Some(queue_id), Some(proposal_id), None, None,
        event_type, actor, Some(role),
        serde_json::json!({"rationale": rationale}),
    ).await?;
    Ok(())
}

/// Notification SLA worker — checks pending reviews and emits alerts.
pub async fn check_review_sla(
    pg: &PgPool, tenant: &TenantId,
    notify_after_hours: i64, sla_hours: i64,
) -> ActiveLearningResult<Vec<NotificationEvent>> {
    let rows: Vec<(Uuid, DateTime<Utc>)> = sqlx::query_as(
        r#"SELECT p.id, p.created_at
           FROM gold_label_proposal p
           JOIN gold_label_queue glq ON glq.id = p.queue_id
           WHERE p.tenant_id = $1
             AND glq.status IN ('LLM_LABELED', 'AWAITING_REVIEW')
             AND p.created_at < NOW() - ($2::TEXT)::INTERVAL"#,
    )
    .bind(&tenant.0).bind(format!("{} hours", notify_after_hours))
    .fetch_all(pg).await?;

    let now = Utc::now();
    let sla_dur = Duration::hours(sla_hours);
    let mut events = Vec::new();
    for (id, created) in rows {
        let age = now - created;
        let level = if age > sla_dur { NotificationLevel::Emergency }
                    else if age > sla_dur * 9 / 10 { NotificationLevel::Escalate }
                    else { NotificationLevel::Notify };
        events.push(NotificationEvent { proposal_id: id, age_hours: age.num_hours(), level });
    }
    Ok(events)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel { Notify, Escalate, Emergency }

#[derive(Debug, Clone)]
pub struct NotificationEvent {
    pub proposal_id: Uuid,
    pub age_hours: i64,
    pub level: NotificationLevel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_serialization() {
        assert_eq!(ReviewDecision::Confirm.as_str(), "CONFIRM");
        assert_eq!(ReviewDecision::EditAndConfirm.as_str(), "EDIT_AND_CONFIRM");
        assert_eq!(ReviewDecision::Reject.as_str(), "REJECT");
    }

    #[test]
    fn maps_review_source() {
        assert_eq!(map_review_source_to_gold("human_review_confirm"), "human_curated");
        assert_eq!(map_review_source_to_gold("auto_confirmed"), "production_overflow");
    }
}
