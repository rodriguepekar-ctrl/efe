//! Rollout / canary / rollback engine.
//!
//! Promotes an APPROVED proposal into a CANDIDATE taxonomy version, runs
//! a shadow canary at N% traffic, then auto-promotes or auto-rollbacks
//! based on metrics. Manual rollback is also supported.
//!
//! See § 12 of `EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md`.

use crate::{CurationError, CurationResult, TenantId, CatalogId};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanaryConfig {
    pub traffic_percent: f32,
    pub duration_hours: u32,
    pub accuracy_pass_threshold_pp: f32,
    pub latency_pass_threshold_ms: u32,
    pub error_rate_pass_threshold: f32,
    pub min_canary_samples: u64,
}

impl Default for CanaryConfig {
    fn default() -> Self {
        Self {
            traffic_percent: 0.05,
            duration_hours: 24,
            accuracy_pass_threshold_pp: -0.5,
            latency_pass_threshold_ms: 50,
            error_rate_pass_threshold: 0.001,
            min_canary_samples: 1000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanaryMetrics {
    pub n_samples: u64,
    pub accuracy_delta_pp: f32,
    pub latency_p99_delta_ms: i32,
    pub error_rate_delta: f32,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanaryDecision { Promote, Rollback(String) }

/// Evaluate canary metrics against config thresholds.
pub fn decide_canary(metrics: &CanaryMetrics, cfg: &CanaryConfig) -> CanaryDecision {
    if metrics.n_samples < cfg.min_canary_samples {
        return CanaryDecision::Rollback(format!(
            "insufficient samples : {} < {}", metrics.n_samples, cfg.min_canary_samples
        ));
    }
    if metrics.accuracy_delta_pp < cfg.accuracy_pass_threshold_pp {
        return CanaryDecision::Rollback(format!(
            "accuracy regression : {:.2} pp < threshold {:.2} pp",
            metrics.accuracy_delta_pp, cfg.accuracy_pass_threshold_pp
        ));
    }
    if metrics.latency_p99_delta_ms > cfg.latency_pass_threshold_ms as i32 {
        return CanaryDecision::Rollback(format!(
            "latency regression : p99 +{} ms > threshold +{} ms",
            metrics.latency_p99_delta_ms, cfg.latency_pass_threshold_ms
        ));
    }
    if metrics.error_rate_delta > cfg.error_rate_pass_threshold {
        return CanaryDecision::Rollback(format!(
            "error rate regression : +{:.4} > threshold +{:.4}",
            metrics.error_rate_delta, cfg.error_rate_pass_threshold
        ));
    }
    CanaryDecision::Promote
}

/// Acquire an advisory lock per (tenant, catalog) to serialize promotes.
async fn acquire_promote_lock(
    tx: &mut Transaction<'_, Postgres>, tenant: &TenantId, catalog: &CatalogId,
) -> CurationResult<()> {
    let key = format!("efe-promote:{}:{}", tenant.0, catalog.0);
    let key_hash: i64 = {
        use sha2::{Digest, Sha256};
        let h = Sha256::digest(key.as_bytes());
        i64::from_le_bytes(h[..8].try_into().unwrap())
    };
    let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
        .bind(key_hash)
        .fetch_one(&mut **tx).await?;
    if !acquired {
        return Err(CurationError::AdvisoryLockContention {
            tenant: tenant.0.clone(), catalog: catalog.0.clone(),
        });
    }
    Ok(())
}

/// Promote an APPROVED proposal :
///   1. acquire advisory lock
///   2. verify proposal status + 4-eyes if required
///   3. demote previous ACTIVE -> DEPRECATED
///   4. create a new taxonomy_version with status CANDIDATE
///   5. record canary_started_at
pub async fn start_canary(
    pg: &PgPool, proposal_id: Uuid, tenant: &TenantId, catalog: &CatalogId,
    require_4_eyes: bool, candidate: crate::version_dag::TaxonomyVersion,
) -> CurationResult<Uuid> {
    let mut tx = pg.begin().await?;
    acquire_promote_lock(&mut tx, tenant, catalog).await?;

    // Check proposal status + 4-eyes (runtime query — see proposal::transition note)
    let proposal_status: Option<crate::proposal::ProposalStatus> = sqlx::query_scalar(
        r#"SELECT status FROM taxonomy_change_proposal WHERE id = $1"#,
    )
    .bind(proposal_id)
    .fetch_optional(&mut *tx).await?;
    let proposal_status = proposal_status.ok_or(CurationError::ProposalNotFound(proposal_id))?;
    if proposal_status != crate::proposal::ProposalStatus::Approved {
        return Err(CurationError::InvalidTransition {
            from: format!("{:?}", proposal_status), to: "Promoted".into(),
        });
    }
    if require_4_eyes {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT reviewer_oidc_sub) FROM curation_review WHERE proposal_id = $1 AND decision = 'APPROVE'"
        )
        .bind(proposal_id)
        .fetch_one(&mut *tx).await?;
        if count < 2 {
            return Err(CurationError::FourEyesRequired { actual: count as usize });
        }
    }

    // Insert candidate version
    crate::version_dag::insert_version(&mut tx, &candidate).await?;

    // Mark canary start on the version
    sqlx::query("UPDATE taxonomy_version SET canary_started_at = NOW(), status = 'CANDIDATE'::taxonomy_version_status WHERE id = $1")
        .bind(candidate.id)
        .execute(&mut *tx).await?;

    // Link proposal to candidate
    sqlx::query("UPDATE taxonomy_change_proposal SET candidate_taxonomy_version_id = $1, updated_at = NOW() WHERE id = $2")
        .bind(candidate.id).bind(proposal_id)
        .execute(&mut *tx).await?;

    tx.commit().await?;
    Ok(candidate.id)
}

/// Finalize canary : called after `duration_hours` have elapsed by a scheduled job.
pub async fn finalize_canary(
    pg: &PgPool, candidate_version_id: Uuid, tenant: &TenantId, catalog: &CatalogId,
    metrics: CanaryMetrics, cfg: &CanaryConfig,
) -> CurationResult<CanaryDecision> {
    let decision = decide_canary(&metrics, cfg);
    let mut tx = pg.begin().await?;
    acquire_promote_lock(&mut tx, tenant, catalog).await?;

    // Persist metrics regardless
    let metrics_json = serde_json::to_value(&metrics)?;
    sqlx::query(
        r#"UPDATE taxonomy_version
           SET canary_completed_at = NOW(), canary_metrics_json = $1
           WHERE id = $2"#,
    )
    .bind(&metrics_json).bind(candidate_version_id)
    .execute(&mut *tx).await?;

    match &decision {
        CanaryDecision::Promote => {
            // Demote previous ACTIVE
            sqlx::query(
                r#"UPDATE taxonomy_version
                   SET status = 'DEPRECATED'::taxonomy_version_status
                   WHERE tenant_id = $1 AND catalog_id = $2 AND status = 'ACTIVE'"#,
            )
            .bind(&tenant.0).bind(&catalog.0)
            .execute(&mut *tx).await?;

            // Promote candidate -> ACTIVE
            sqlx::query(
                r#"UPDATE taxonomy_version
                   SET status = 'ACTIVE'::taxonomy_version_status, promoted_at = NOW()
                   WHERE id = $1"#,
            )
            .bind(candidate_version_id)
            .execute(&mut *tx).await?;

            // Mark proposal PROMOTED
            sqlx::query(
                r#"UPDATE taxonomy_change_proposal
                   SET status = 'PROMOTED'::proposal_status, updated_at = NOW()
                   WHERE candidate_taxonomy_version_id = $1"#,
            )
            .bind(candidate_version_id)
            .execute(&mut *tx).await?;
        }
        CanaryDecision::Rollback(_reason) => {
            // Rollback candidate -> ROLLED_BACK
            sqlx::query(
                r#"UPDATE taxonomy_version
                   SET status = 'ROLLED_BACK'::taxonomy_version_status, rolled_back_at = NOW()
                   WHERE id = $1"#,
            )
            .bind(candidate_version_id)
            .execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    Ok(decision)
}

/// Manual rollback — operator-triggered.
///
/// Inserts a NEW taxonomy_version row that is a copy of the version
/// pre-promote (i.e. ACTIVE before the bad promote). The current ACTIVE
/// becomes ROLLED_BACK ; the new version becomes ACTIVE.
pub async fn manual_rollback(
    pg: &PgPool, target_version_id: Uuid, tenant: &TenantId, catalog: &CatalogId,
    reason: &str, actor_id: &str,
) -> CurationResult<Uuid> {
    if reason.len() < 10 {
        return Err(CurationError::InvalidInput("rollback reason must be ≥ 10 chars".into()));
    }
    let mut tx = pg.begin().await?;
    acquire_promote_lock(&mut tx, tenant, catalog).await?;

    // Fetch target version (the one to roll back to) — runtime query
    let target_row: Option<(String, serde_json::Value)> = sqlx::query_as(
        r#"SELECT semver, anchors_json
           FROM taxonomy_version
           WHERE id = $1 AND tenant_id = $2 AND catalog_id = $3"#,
    )
    .bind(target_version_id).bind(&tenant.0).bind(&catalog.0)
    .fetch_optional(&mut *tx).await?;
    let (target_semver, target_anchors) =
        target_row.ok_or(CurationError::TaxonomyVersionNotFound(target_version_id))?;

    // Compute new semver : bump patch on target
    let mut sv = crate::version_dag::SemVer::parse(&target_semver)?;
    sv.patch += 1;
    let new_semver = format!("{}.{}.{}", sv.major, sv.minor, sv.patch);

    let new_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO taxonomy_version
           (id, tenant_id, catalog_id, semver, parent_id, status, anchors_json, created_at)
           VALUES ($1, $2, $3, $4, $5, 'ACTIVE'::taxonomy_version_status, $6, NOW())"#,
    )
    .bind(new_id).bind(&tenant.0).bind(&catalog.0).bind(&new_semver)
    .bind(target_version_id).bind(&target_anchors)
    .execute(&mut *tx).await?;

    // Mark current ACTIVE as ROLLED_BACK (excluding the new one we just inserted)
    sqlx::query(
        r#"UPDATE taxonomy_version
           SET status = 'ROLLED_BACK'::taxonomy_version_status, rolled_back_at = NOW()
           WHERE tenant_id = $1 AND catalog_id = $2 AND status = 'ACTIVE' AND id != $3"#,
    )
    .bind(&tenant.0).bind(&catalog.0).bind(new_id)
    .execute(&mut *tx).await?;

    // Audit row
    crate::audit::append_audit(
        &mut tx, tenant, None, Some(new_id),
        "rollback_manual", actor_id, None,
        serde_json::json!({
            "target_version": target_version_id,
            "new_version": new_id,
            "reason": reason,
        }),
    ).await?;

    tx.commit().await?;
    Ok(new_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(n: u64, acc_pp: f32, lat: i32, err: f32) -> CanaryMetrics {
        CanaryMetrics {
            n_samples: n, accuracy_delta_pp: acc_pp,
            latency_p99_delta_ms: lat, error_rate_delta: err,
            started_at: Utc::now() - Duration::hours(24),
            completed_at: Some(Utc::now()),
        }
    }

    #[test]
    fn decide_promote_happy_path() {
        let cfg = CanaryConfig::default();
        let m = metrics(2000, 0.5, 10, 0.0001);
        assert_eq!(decide_canary(&m, &cfg), CanaryDecision::Promote);
    }

    #[test]
    fn decide_rollback_insufficient_samples() {
        let cfg = CanaryConfig::default();
        let m = metrics(500, 1.0, 0, 0.0);
        assert!(matches!(decide_canary(&m, &cfg), CanaryDecision::Rollback(_)));
    }

    #[test]
    fn decide_rollback_accuracy_regression() {
        let cfg = CanaryConfig::default();
        let m = metrics(2000, -1.0, 0, 0.0);
        assert!(matches!(decide_canary(&m, &cfg), CanaryDecision::Rollback(_)));
    }

    #[test]
    fn decide_rollback_latency_regression() {
        let cfg = CanaryConfig::default();
        let m = metrics(2000, 0.5, 100, 0.0);
        assert!(matches!(decide_canary(&m, &cfg), CanaryDecision::Rollback(_)));
    }

    #[test]
    fn decide_rollback_error_rate_regression() {
        let cfg = CanaryConfig::default();
        let m = metrics(2000, 0.5, 0, 0.01);
        assert!(matches!(decide_canary(&m, &cfg), CanaryDecision::Rollback(_)));
    }

    #[test]
    fn decide_acceptable_minor_regression() {
        let cfg = CanaryConfig { accuracy_pass_threshold_pp: -1.0, ..Default::default() };
        let m = metrics(2000, -0.7, 0, 0.0);
        assert_eq!(decide_canary(&m, &cfg), CanaryDecision::Promote);
    }
}
