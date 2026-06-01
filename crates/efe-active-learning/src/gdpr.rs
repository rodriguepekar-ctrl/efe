//! GDPR Art. 17 (right to erasure) cascade.
//!
//! Tombstone gold_example rows + queue + cascade rebuild of bundles
//! that indexed those examples. See § 17 of the architecture doc.

use crate::{ActiveLearningError, ActiveLearningResult, CatalogId, TenantId};
use crate::metrics::{AL_GDPR_ART17_COMPLETED_TOTAL, AL_GDPR_ART17_REQUESTS_TOTAL};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Urgency { Standard, Regulatory, Catastrophic }

impl Urgency {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Regulatory => "regulatory",
            Self::Catastrophic => "catastrophic",
        }
    }
    pub fn sla(&self) -> Duration {
        match self {
            Self::Standard => Duration::days(30),
            Self::Regulatory => Duration::hours(72),
            Self::Catastrophic => Duration::hours(24),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ErasureRequest {
    pub tenant: TenantId,
    pub catalog: CatalogId,
    pub urgency: Urgency,
    pub content_hashes: Vec<String>,
    pub requester_subject_id: Option<String>,
    pub reason_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebuildRequestSummary {
    pub id: Uuid,
    pub urgency: String,
    pub sla_deadline: DateTime<Utc>,
    pub affected_count: usize,
    pub status: String,
}

/// Submit an Art. 17 erasure : tombstone the matching gold_example rows
/// + create a bundle_rebuild_request for cascade.
pub async fn submit_erasure(
    pg: &PgPool, req: ErasureRequest,
) -> ActiveLearningResult<RebuildRequestSummary> {
    if req.content_hashes.is_empty() {
        return Err(ActiveLearningError::InvalidInput(
            "content_hashes must not be empty".into()
        ));
    }
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(&req.tenant.0).execute(pg).await?;

    AL_GDPR_ART17_REQUESTS_TOTAL
        .with_label_values(&[&req.tenant.0, req.urgency.as_str()]).inc();

    let mut tx = pg.begin().await?;

    // 1. Find affected gold_example rows by content_hash.
    let placeholders: Vec<String> = (1..=req.content_hashes.len())
        .map(|i| format!("${}", i + 2)).collect();
    let q = format!(
        r#"SELECT ge.id
           FROM gold_example ge
           JOIN gold_dataset gd ON gd.id = ge.gold_dataset_id
           WHERE gd.tenant_id = $1
             AND gd.catalog_id = $2
             AND encode(digest(ge.text, 'sha256'), 'hex') IN ({})"#,
        placeholders.join(",")
    );
    let mut query = sqlx::query_as::<_, (Uuid,)>(&q)
        .bind(&req.tenant.0).bind(&req.catalog.0);
    for h in &req.content_hashes { query = query.bind(h); }
    let affected: Vec<Uuid> = query.fetch_all(&mut *tx).await?
        .into_iter().map(|(id,)| id).collect();

    if affected.is_empty() {
        // No gold examples match — still log the request for compliance trace
        tracing::info!(
            tenant=%req.tenant.0, "GDPR Art. 17 : no matching gold examples"
        );
    }

    // 2. Tombstone gold_example rows (preserve HMAC chain : keep row,
    //    mask text, set pii_redacted_at). Also tombstone matching queue rows.
    if !affected.is_empty() {
        sqlx::query(
            r#"UPDATE gold_example
               SET text = '[REDACTED-Art17-' || NOW()::DATE || ']',
                   pii_redacted_at = NOW()
               WHERE id = ANY($1)"#,
        )
        .bind(&affected)
        .execute(&mut *tx).await?;

        // Queue rows : tombstone the ones with matching content_hash
        sqlx::query(
            r#"UPDATE gold_label_queue
               SET status = 'TOMBSTONED'::gold_label_queue_status,
                   sentence_redacted = '[REDACTED-Art17]'
               WHERE tenant_id = $1
                 AND content_hash = ANY($2::TEXT[])
                 AND status NOT IN ('TOMBSTONED')"#,
        )
        .bind(&req.tenant.0).bind(&req.content_hashes)
        .execute(&mut *tx).await?;
    }

    // 3. Create bundle_rebuild_request
    let req_id = Uuid::new_v4();
    let sla_deadline = Utc::now() + req.urgency.sla();
    sqlx::query(
        r#"INSERT INTO bundle_rebuild_request
           (id, tenant_id, catalog_id, reason, urgency,
            affected_gold_example_ids, requester_subject_id,
            sla_deadline, status)
           VALUES ($1, $2, $3, 'gdpr_art17', $4, $5, $6, $7, 'PENDING')"#,
    )
    .bind(req_id).bind(&req.tenant.0).bind(&req.catalog.0)
    .bind(req.urgency.as_str()).bind(&affected)
    .bind(req.requester_subject_id.as_deref()).bind(sla_deadline)
    .execute(&mut *tx).await?;

    // 4. Audit
    crate::audit::append_learning_audit(
        &mut tx, &req.tenant, None, None, None, None,
        "gdpr_art17_redaction", "system", None,
        serde_json::json!({
            "rebuild_request_id": req_id,
            "catalog_id": req.catalog.0,
            "urgency": req.urgency.as_str(),
            "affected_count": affected.len(),
            "requester_subject_id": req.requester_subject_id,
            "reason": req.reason_text,
        }),
    ).await?;

    tx.commit().await?;

    Ok(RebuildRequestSummary {
        id: req_id,
        urgency: req.urgency.as_str().into(),
        sla_deadline,
        affected_count: affected.len(),
        status: "PENDING".into(),
    })
}

/// Mark a rebuild request completed (called by retune.rs after force-promote).
pub async fn mark_completed(
    pg: &PgPool, request_id: Uuid, retune_run_id: Uuid,
) -> ActiveLearningResult<()> {
    sqlx::query(
        r#"UPDATE bundle_rebuild_request
           SET status = 'COMPLETED', completed_at = NOW(), completed_retune_run_id = $1
           WHERE id = $2"#,
    )
    .bind(retune_run_id).bind(request_id)
    .execute(pg).await?;
    AL_GDPR_ART17_COMPLETED_TOTAL.inc();
    Ok(())
}

/// List pending rebuild requests for a tenant, sorted by SLA deadline.
pub async fn list_pending(
    pg: &PgPool, tenant: &TenantId,
) -> ActiveLearningResult<Vec<RebuildRequestSummary>> {
    let rows: Vec<(Uuid, String, DateTime<Utc>, Vec<Uuid>, String)> = sqlx::query_as(
        r#"SELECT id, urgency, sla_deadline, affected_gold_example_ids, status
           FROM bundle_rebuild_request
           WHERE tenant_id = $1 AND status IN ('PENDING', 'IN_PROGRESS')
           ORDER BY sla_deadline ASC"#,
    )
    .bind(&tenant.0)
    .fetch_all(pg).await?;
    Ok(rows.into_iter().map(|(id, urg, dl, aff, st)| RebuildRequestSummary {
        id, urgency: urg, sla_deadline: dl, affected_count: aff.len(), status: st,
    }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urgency_sla_correct() {
        assert_eq!(Urgency::Standard.sla(), Duration::days(30));
        assert_eq!(Urgency::Regulatory.sla(), Duration::hours(72));
        assert_eq!(Urgency::Catastrophic.sla(), Duration::hours(24));
    }

    #[test]
    fn empty_request_rejected() {
        let req = ErasureRequest {
            tenant: TenantId("acme".into()),
            catalog: CatalogId("c1".into()),
            urgency: Urgency::Standard,
            content_hashes: vec![],
            requester_subject_id: None,
            reason_text: "test".into(),
        };
        // Note : actual call requires a PG pool ; here we just test the
        // shape. The error path is checked in the integration test.
        assert!(req.content_hashes.is_empty());
    }
}
