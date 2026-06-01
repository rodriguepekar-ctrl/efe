//! Bundle re-tune orchestrator. Periodic (weekly default) per-catalog,
//! produces a new bundle artifact via centroid recompute, runs eval,
//! flags for promotion if accuracy delta ≥ +1pp.
//!
//! See § 12 of EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md.

use crate::{
    audit, holdout,
    ActiveLearningError, ActiveLearningResult, CatalogId, TenantId,
};
use crate::metrics::{AL_CENTROID_DRIFT_ABORTED_TOTAL, AL_RETUNE_RUNS_TOTAL};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "retune_run_status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RetuneRunStatus {
    Tuning,
    EvalRunning,
    EvalPass,
    EvalFail,
    Promoted,
    RolledBack,
    Aborted,
}

impl RetuneRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tuning => "TUNING",
            Self::EvalRunning => "EVAL_RUNNING",
            Self::EvalPass => "EVAL_PASS",
            Self::EvalFail => "EVAL_FAIL",
            Self::Promoted => "PROMOTED",
            Self::RolledBack => "ROLLED_BACK",
            Self::Aborted => "ABORTED",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetuneRun {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub catalog_id: CatalogId,
    pub parent_taxonomy_version_id: Uuid,
    pub new_taxonomy_version_id: Option<Uuid>,
    pub new_bundle_semver: String,
    pub gold_dataset_id: Uuid,
    pub training_examples_count: i64,
    pub holdout_examples_count: i64,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status: RetuneRunStatus,
    pub eval_metrics_json: Option<serde_json::Value>,
    pub flagged_for_promotion: bool,
    pub triggered_by: String,
    pub triggered_by_actor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RetuneTrigger {
    pub kind: TriggerKind,
    pub actor: Option<String>,
}

#[derive(Debug, Clone)]
pub enum TriggerKind { Scheduler, OnDemand, GdprCascade }

impl TriggerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Scheduler => "scheduler",
            Self::OnDemand => "on_demand",
            Self::GdprCascade => "gdpr_cascade",
        }
    }
}

/// Insert a new retune_run row in TUNING status.
pub async fn insert_retune_run(
    pg: &PgPool,
    tenant: &TenantId, catalog: &CatalogId,
    parent_taxonomy_version_id: Uuid,
    new_bundle_semver: &str,
    gold_dataset_id: Uuid,
    training_count: i64, holdout_count: i64,
    trigger: &RetuneTrigger,
) -> ActiveLearningResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO bundle_retune_run
           (id, tenant_id, catalog_id, parent_taxonomy_version_id,
            new_bundle_semver, gold_dataset_id,
            training_examples_count, holdout_examples_count,
            started_at, status, flagged_for_promotion,
            triggered_by, triggered_by_actor)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW(),
                   'TUNING'::retune_run_status, FALSE, $9, $10)"#,
    )
    .bind(id).bind(&tenant.0).bind(&catalog.0)
    .bind(parent_taxonomy_version_id)
    .bind(new_bundle_semver).bind(gold_dataset_id)
    .bind(training_count).bind(holdout_count)
    .bind(trigger.kind.as_str()).bind(trigger.actor.as_deref())
    .execute(pg).await?;
    Ok(id)
}

/// Transition retune_run status with metric emission.
pub async fn transition_status(
    pg: &PgPool, run_id: Uuid, tenant: &TenantId, new_status: RetuneRunStatus,
    eval_metrics: Option<serde_json::Value>, flagged: bool,
) -> ActiveLearningResult<()> {
    let mut tx = pg.begin().await?;
    sqlx::query(
        r#"UPDATE bundle_retune_run
           SET status = $1::retune_run_status,
               completed_at = CASE WHEN $1 IN ('EVAL_PASS', 'EVAL_FAIL', 'PROMOTED', 'ROLLED_BACK', 'ABORTED')
                                   THEN NOW() ELSE completed_at END,
               eval_metrics_json = COALESCE($2, eval_metrics_json),
               flagged_for_promotion = $3
           WHERE id = $4"#,
    )
    .bind(new_status.as_str()).bind(&eval_metrics).bind(flagged).bind(run_id)
    .execute(&mut *tx).await?;

    audit::append_learning_audit(
        &mut tx, tenant, None, None, Some(run_id), None,
        match new_status {
            RetuneRunStatus::EvalPass => "retune_eval_pass",
            RetuneRunStatus::EvalFail => "retune_eval_fail",
            RetuneRunStatus::Aborted  => "retune_aborted",
            RetuneRunStatus::Promoted => "promote_completed",
            RetuneRunStatus::RolledBack => "promote_rolled_back",
            RetuneRunStatus::Tuning   => "retune_started",
            RetuneRunStatus::EvalRunning => "retune_started",
        },
        "system", None,
        serde_json::json!({"run_id": run_id, "status": new_status.as_str(), "flagged": flagged}),
    ).await?;

    AL_RETUNE_RUNS_TOTAL
        .with_label_values(&[&tenant.0, new_status.as_str()]).inc();
    tx.commit().await?;
    Ok(())
}

/// Abort a run with reason — used when centroid drift threshold trips.
pub async fn abort_with_drift(
    pg: &PgPool, run_id: Uuid, tenant: &TenantId, catalog: &CatalogId,
    drift: f32, max: f32,
) -> ActiveLearningResult<()> {
    AL_CENTROID_DRIFT_ABORTED_TOTAL
        .with_label_values(&[&tenant.0, &catalog.0]).inc();
    let mut tx = pg.begin().await?;
    sqlx::query(
        r#"UPDATE bundle_retune_run
           SET status = 'ABORTED'::retune_run_status, completed_at = NOW()
           WHERE id = $1"#,
    )
    .bind(run_id).execute(&mut *tx).await?;
    audit::append_learning_audit(
        &mut tx, tenant, None, None, Some(run_id), None,
        "centroid_drift_aborted", "system", None,
        serde_json::json!({
            "run_id": run_id, "drift": drift, "max": max,
            "reason": "centroid drift exceeded threshold (likely poisoning)"
        }),
    ).await?;
    tx.commit().await?;
    Ok(())
}

/// Check if a (tenant, catalog) is due for scheduled retune.
pub async fn is_due_for_retune(
    pg: &PgPool, tenant: &TenantId, catalog: &CatalogId, interval_days: i64,
) -> ActiveLearningResult<bool> {
    let last_completed: Option<DateTime<Utc>> = sqlx::query_scalar(
        r#"SELECT MAX(completed_at) FROM bundle_retune_run
           WHERE tenant_id = $1 AND catalog_id = $2
             AND status IN ('EVAL_PASS', 'EVAL_FAIL', 'PROMOTED', 'ABORTED')"#,
    )
    .bind(&tenant.0).bind(&catalog.0)
    .fetch_one(pg).await?;
    Ok(match last_completed {
        None => true,
        Some(t) => (Utc::now() - t).num_days() >= interval_days,
    })
}

/// Compute number of new gold examples since the last retune for
/// gating the threshold check.
pub async fn count_new_examples_since_last_retune(
    pg: &PgPool, tenant: &TenantId, catalog: &CatalogId,
) -> ActiveLearningResult<u64> {
    let last_completed: Option<DateTime<Utc>> = sqlx::query_scalar(
        r#"SELECT MAX(completed_at) FROM bundle_retune_run
           WHERE tenant_id = $1 AND catalog_id = $2
             AND status IN ('EVAL_PASS', 'EVAL_FAIL', 'PROMOTED')"#,
    )
    .bind(&tenant.0).bind(&catalog.0)
    .fetch_one(pg).await?;

    let cutoff = last_completed.unwrap_or_else(|| Utc::now() - Duration::days(365));
    let count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM gold_example ge
           JOIN gold_dataset gd ON gd.id = ge.gold_dataset_id
           WHERE gd.tenant_id = $1 AND gd.catalog_id = $2
             AND ge.created_at > $3
             AND ge.pii_redacted_at IS NULL"#,
    )
    .bind(&tenant.0).bind(&catalog.0).bind(cutoff)
    .fetch_one(pg).await?;
    Ok(count as u64)
}

/// Bump patch on semver string : "3.2.0" → "3.2.1".
///
/// **M30 audit fix** — also validates that `major` and `minor` parse
/// as `u32`. Previously, inputs like `v1.2.3` or `1.x.3` slipped
/// through because only `patch` was checked. The `bump_patch_invalid`
/// test that asserted those inputs error out had been failing
/// silently ; this fix re-aligns the implementation with the
/// pinned contract.
pub fn bump_patch(semver: &str) -> ActiveLearningResult<String> {
    let parts: Vec<&str> = semver.split('.').collect();
    if parts.len() != 3 {
        return Err(ActiveLearningError::InvalidInput(
            format!("not a semver triple : '{}'", semver)
        ));
    }
    let major: u32 = parts[0].parse()
        .map_err(|_| ActiveLearningError::InvalidInput(format!("bad major : '{}'", parts[0])))?;
    let minor: u32 = parts[1].parse()
        .map_err(|_| ActiveLearningError::InvalidInput(format!("bad minor : '{}'", parts[1])))?;
    let patch: u32 = parts[2].parse()
        .map_err(|_| ActiveLearningError::InvalidInput(format!("bad patch : '{}'", parts[2])))?;
    Ok(format!("{}.{}.{}", major, minor, patch + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_patch_ok() {
        assert_eq!(bump_patch("3.2.0").unwrap(), "3.2.1");
        assert_eq!(bump_patch("0.0.0").unwrap(), "0.0.1");
        assert_eq!(bump_patch("1.0.99").unwrap(), "1.0.100");
    }

    #[test]
    fn bump_patch_invalid() {
        assert!(bump_patch("1.2").is_err());
        assert!(bump_patch("v1.2.3").is_err());
        assert!(bump_patch("a.b.c").is_err());
    }

    #[test]
    fn status_serialization() {
        assert_eq!(RetuneRunStatus::EvalPass.as_str(), "EVAL_PASS");
        assert_eq!(RetuneRunStatus::RolledBack.as_str(), "ROLLED_BACK");
    }
}
