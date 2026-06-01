//! Uncertainty sampler — selects low-confidence production classifies
//! for LLM labeling.
//!
//! Architecture : scheduled scan every 30s (configurable). Per-tenant
//! per-catalog queries with content-hash dedup + back-pressure.
//!
//! See § 7 of EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md.

use crate::{ActiveLearningError, ActiveLearningResult, CatalogId, TenantId};
use crate::metrics::{AL_QUEUE_BACK_PRESSURE_TOTAL, AL_SAMPLES_ENQUEUED_TOTAL};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SamplerConfig {
    pub uncertainty_threshold: f32,
    pub queue_max_size: u32,
    pub dedup_window_days: u32,
    pub batch_scan_limit: u32,
    pub lookback_days: u32,
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            uncertainty_threshold: 0.70,
            queue_max_size: 10_000,
            dedup_window_days: 30,
            batch_scan_limit: 1_000,
            lookback_days: 7,
        }
    }
}

/// Candidate row read from llm_call_audit.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CandidateSample {
    pub llm_call_id: Uuid,
    pub tenant_id: String,
    pub catalog_id: String,
    pub sentence_redacted: String,
    pub proposed_class_id: Option<String>,
    pub confidence: f64,
    pub content_hash: String,
}

/// Find low-confidence samples not yet enqueued and not in gold dataset.
///
/// Uses Postgres `digest()` for content hashing. The query is scoped to
/// the tenant + catalog and applies dedup against `gold_label_queue` and
/// `gold_example`.
pub async fn select_candidates(
    pg: &PgPool, tenant: &TenantId, catalog: &CatalogId, cfg: &SamplerConfig,
) -> ActiveLearningResult<Vec<CandidateSample>> {
    let rows = sqlx::query_as::<_, CandidateSample>(
        r#"
        SELECT
            llc.id           AS llm_call_id,
            llc.tenant_id    AS tenant_id,
            llc.catalog_id   AS catalog_id,
            llc.sentence_redacted AS sentence_redacted,
            llc.verdict_class_id  AS proposed_class_id,
            llc.confidence   AS confidence,
            encode(digest(llc.sentence_redacted, 'sha256'), 'hex') AS content_hash
        FROM llm_call_audit llc
        WHERE llc.tenant_id = $1
          AND llc.catalog_id = $2
          AND llc.confidence < $3
          AND llc.created_at > NOW() - ($4::TEXT)::INTERVAL
          AND NOT EXISTS (
              SELECT 1 FROM gold_label_queue glq
              WHERE glq.tenant_id = llc.tenant_id
                AND glq.content_hash = encode(digest(llc.sentence_redacted, 'sha256'), 'hex')
                AND glq.enqueued_at > NOW() - ($5::TEXT)::INTERVAL
          )
          AND NOT EXISTS (
              SELECT 1 FROM gold_example ge
              JOIN gold_dataset gd ON gd.id = ge.gold_dataset_id
              WHERE gd.tenant_id = llc.tenant_id
                AND gd.catalog_id = llc.catalog_id
                AND encode(digest(ge.text, 'sha256'), 'hex') =
                    encode(digest(llc.sentence_redacted, 'sha256'), 'hex')
          )
        ORDER BY llc.created_at DESC
        LIMIT $6
        "#,
    )
    .bind(&tenant.0).bind(&catalog.0)
    .bind(cfg.uncertainty_threshold as f64)
    .bind(format!("{} days", cfg.lookback_days))
    .bind(format!("{} days", cfg.dedup_window_days))
    .bind(cfg.batch_scan_limit as i64)
    .fetch_all(pg).await?;
    Ok(rows)
}

/// Compute available room in the queue (max - in-flight).
pub async fn available_capacity(
    pg: &PgPool, tenant: &TenantId, max_size: u32,
) -> ActiveLearningResult<u32> {
    let in_flight: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM gold_label_queue
           WHERE tenant_id = $1
             AND status IN ('ENQUEUED', 'LABELING', 'LLM_LABELED', 'AWAITING_REVIEW')"#,
    )
    .bind(&tenant.0)
    .fetch_one(pg).await?;
    Ok((max_size as i64 - in_flight).max(0) as u32)
}

/// Bulk-insert candidates into the queue with ON CONFLICT DO NOTHING
/// (handles race between scanner and concurrent inserts).
pub async fn enqueue_batch(
    pg: &PgPool, candidates: &[CandidateSample],
) -> ActiveLearningResult<u64> {
    if candidates.is_empty() { return Ok(0); }
    let mut inserted = 0u64;
    let mut tx = pg.begin().await?;
    for c in candidates {
        let res = sqlx::query(
            r#"INSERT INTO gold_label_queue
               (id, tenant_id, catalog_id, content_hash, sentence_redacted,
                proposed_class_id, observed_confidence, status, source_llm_call_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7,
                       'ENQUEUED'::gold_label_queue_status, $8)
               ON CONFLICT (tenant_id, content_hash) DO NOTHING"#,
        )
        .bind(Uuid::new_v4())
        .bind(&c.tenant_id).bind(&c.catalog_id)
        .bind(&c.content_hash).bind(&c.sentence_redacted)
        .bind(&c.proposed_class_id).bind(c.confidence)
        .bind(c.llm_call_id)
        .execute(&mut *tx).await?;
        inserted += res.rows_affected();
    }
    tx.commit().await?;
    Ok(inserted)
}

/// One-tick scan : combines select + capacity + enqueue.
pub async fn run_one_scan(
    pg: &PgPool, tenant: &TenantId, catalog: &CatalogId, cfg: &SamplerConfig,
) -> ActiveLearningResult<u64> {
    // Set RLS session
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(&tenant.0).execute(pg).await?;

    let candidates = select_candidates(pg, tenant, catalog, cfg).await?;
    if candidates.is_empty() { return Ok(0); }

    let avail = available_capacity(pg, tenant, cfg.queue_max_size).await? as usize;
    if avail == 0 {
        AL_QUEUE_BACK_PRESSURE_TOTAL
            .with_label_values(&[&tenant.0]).inc();
        return Ok(0);
    }

    let to_insert = &candidates[..avail.min(candidates.len())];
    let inserted = enqueue_batch(pg, to_insert).await?;
    AL_SAMPLES_ENQUEUED_TOTAL
        .with_label_values(&[&tenant.0, &catalog.0])
        .inc_by(inserted);
    Ok(inserted)
}

/// Trait for tenant registry (mockable in tests).
#[async_trait::async_trait]
pub trait TenantRegistry: Send + Sync {
    async fn iter_active_with_al(&self) -> ActiveLearningResult<Vec<TenantContext>>;
}

pub struct TenantContext {
    pub id: TenantId,
    pub catalogs: Vec<CatalogContext>,
    pub config: SamplerConfig,
}

pub struct CatalogContext { pub id: CatalogId, pub al_enabled: bool }

/// Long-running scheduler : runs `run_one_scan` for every active
/// (tenant, catalog) pair every `interval_secs` seconds.
pub async fn scheduler_loop(
    pg: Arc<PgPool>, tenants: Arc<dyn TenantRegistry>, interval_secs: u64,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
    loop {
        interval.tick().await;
        let tenants_snapshot = match tenants.iter_active_with_al().await {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(?e, "sampler scheduler tenant fetch failed");
                continue;
            }
        };
        for tc in tenants_snapshot {
            for cat in tc.catalogs.iter().filter(|c| c.al_enabled) {
                if let Err(e) = run_one_scan(&pg, &tc.id, &cat.id, &tc.config).await {
                    tracing::error!(?e, tenant=%tc.id.0, catalog=%cat.id.0, "sampler scan failed");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_sane() {
        let cfg = SamplerConfig::default();
        assert_eq!(cfg.uncertainty_threshold, 0.70);
        assert_eq!(cfg.queue_max_size, 10_000);
    }
}
