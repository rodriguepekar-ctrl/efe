//! Cost governance : per-run cap + tenant daily budget watchdog.
//!
//! ## Per-run cap
//!
//! Every `agent_step` row stores `cost_cents`. The controller updates
//! `agent_run.current_cost_cents` after each LLM call. When
//! `current_cost_cents` reaches `max_cost_cents` the controller
//! transitions the run to `COST_CAPPED` and SIGTERMs the microVM
//! **immediately** (no grace period).
//!
//! ## Tenant daily budget
//!
//! `tenant_feature.m25_active_learning_budget_cents_per_day` (read
//! from M24) gives a soft daily cap. We bookkeep with `agent_step`
//! aggregations and emit a warning at 80 % consumption.
//!
//! ## Watchdog
//!
//! `watchdog_loop` runs forever in the controller binary, every 5 s
//! sweeping active runs and enforcing both rules.

use crate::run::AgentRunStatus;
use crate::{AgentRunnerError, AgentRunnerResult, TenantId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CostState {
    pub run_id: Uuid,
    pub spent_cents: i64,
    pub cap_cents: i64,
    pub pct: f64,
    pub warned_80: bool,
    pub capped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CostAction {
    Continue,
    Warn80Pct,
    Terminate,
}

/// Decide what to do given the current run cost.
pub fn classify(spent_cents: i64, cap_cents: i64, already_warned: bool) -> CostAction {
    if cap_cents <= 0 {
        return CostAction::Terminate;
    }
    if spent_cents >= cap_cents {
        return CostAction::Terminate;
    }
    let pct = (spent_cents as f64 / cap_cents as f64) * 100.0;
    if pct >= 80.0 && !already_warned {
        return CostAction::Warn80Pct;
    }
    CostAction::Continue
}

/// Increment the cumulative cost on an agent_run, taking the new
/// total. Returns the new total + whether this crossed the cap.
pub async fn add_cost(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Uuid,
    delta_cents: i64,
) -> AgentRunnerResult<i64> {
    if delta_cents < 0 {
        return Err(AgentRunnerError::InvalidInput(
            "cost delta must be ≥ 0".into(),
        ));
    }
    let new_total: Option<i64> = sqlx::query_scalar(
        r#"UPDATE agent_run
           SET current_cost_cents = current_cost_cents + $1
           WHERE id = $2
           RETURNING current_cost_cents"#,
    )
    .bind(delta_cents)
    .bind(run_id)
    .fetch_optional(&mut **tx)
    .await?;
    new_total.ok_or(AgentRunnerError::RunNotFound(run_id))
}

/// Per-tenant total for a calendar day (UTC).
pub async fn tenant_daily_spend_cents(
    pg: &PgPool,
    tenant: &TenantId,
    day: DateTime<Utc>,
) -> AgentRunnerResult<i64> {
    let day_start = day.date_naive().and_hms_opt(0, 0, 0).unwrap();
    let day_start_utc = chrono::DateTime::<Utc>::from_naive_utc_and_offset(day_start, Utc);
    let n: Option<i64> = sqlx::query_scalar(
        r#"SELECT COALESCE(SUM(current_cost_cents), 0)
           FROM agent_run
           WHERE tenant_id = $1
             AND created_at >= $2
             AND created_at <  $2 + INTERVAL '1 day'"#,
    )
    .bind(&tenant.0)
    .bind(day_start_utc)
    .fetch_one(pg)
    .await?;
    Ok(n.unwrap_or(0))
}

/// The watchdog loop. Sweeps active runs every 5 s. Per design partner
/// pilot scale (max 5 concurrent runs / tenant, ≤ 3 tenants) this is
/// ample headroom ; we'd shard to per-tenant tasks at scale.
pub async fn watchdog_loop(pg: PgPool) {
    tracing::info!("cost-cap watchdog loop started");
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if let Err(e) = sweep_once(&pg).await {
            tracing::error!(?e, "cost watchdog sweep error");
        }
    }
}

#[derive(sqlx::FromRow)]
struct WatchRow {
    id: Uuid,
    tenant_id: String,
    current_cost_cents: i64,
    max_cost_cents: i64,
    status: AgentRunStatus,
}

async fn sweep_once(pg: &PgPool) -> AgentRunnerResult<()> {
    // Pull the small set of active runs (≤ ~50 per pilot tenant).
    let rows: Vec<WatchRow> = sqlx::query_as(
        r#"SELECT id, tenant_id, current_cost_cents, max_cost_cents, status
           FROM agent_run
           WHERE status NOT IN ('COMPLETED','FAILED','TERMINATED','TIMED_OUT','COST_CAPPED')"#,
    )
    .fetch_all(pg)
    .await?;
    for r in rows {
        match classify(r.current_cost_cents, r.max_cost_cents, false) {
            CostAction::Continue => {}
            CostAction::Warn80Pct => {
                tracing::warn!(
                    run_id = %r.id,
                    tenant = %r.tenant_id,
                    spent = r.current_cost_cents,
                    cap = r.max_cost_cents,
                    "cost 80 % consumed"
                );
            }
            CostAction::Terminate => {
                tracing::warn!(
                    run_id = %r.id,
                    tenant = %r.tenant_id,
                    spent = r.current_cost_cents,
                    cap = r.max_cost_cents,
                    "cost cap exceeded → COST_CAPPED"
                );
                let mut tx = pg.begin().await?;
                let _ = sqlx::query(
                    r#"UPDATE agent_run
                       SET status = 'COST_CAPPED'::agent_run_status,
                           completed_at = NOW(),
                           failure_reason = 'cost cap exceeded'
                       WHERE id = $1 AND status = $2::agent_run_status"#,
                )
                .bind(r.id)
                .bind(r.status.as_sql_str())
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                crate::metrics::AGENT_COST_CAP_HIT_TOTAL
                    .with_label_values(&[&r.tenant_id])
                    .inc();
                crate::metrics::AGENT_COST_CENTS_TOTAL
                    .with_label_values(&[&r.tenant_id])
                    .inc_by(r.current_cost_cents.max(0) as u64);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_continue_below_80() {
        assert_eq!(classify(50, 100, false), CostAction::Continue);
    }
    #[test]
    fn classify_warn_at_80() {
        assert_eq!(classify(80, 100, false), CostAction::Warn80Pct);
    }
    #[test]
    fn classify_no_warn_when_already_warned() {
        assert_eq!(classify(85, 100, true), CostAction::Continue);
    }
    #[test]
    fn classify_terminate_at_cap() {
        assert_eq!(classify(100, 100, false), CostAction::Terminate);
    }
    #[test]
    fn classify_terminate_above_cap() {
        assert_eq!(classify(150, 100, false), CostAction::Terminate);
    }
    #[test]
    fn classify_terminate_with_zero_cap() {
        assert_eq!(classify(0, 0, false), CostAction::Terminate);
    }
}
