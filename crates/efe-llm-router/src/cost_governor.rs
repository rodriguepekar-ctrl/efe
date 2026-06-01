//! Per-tenant cost governance (M21 Phase C).
//!
//! Postgres-backed budget enforcement with `SELECT FOR UPDATE` for
//! race-safe concurrent access across replicas. We use sqlx's
//! runtime-checked queries (no compile-time DB connection required).

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use sqlx::PgPool;
use std::str::FromStr;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct BudgetState {
    pub cap_usd: f64,
    pub spent_usd: f64,
    pub projected_usd: f64,
    pub pct_used: f64,
    pub soft_warning: bool,
    pub fail_open: bool,
}

#[derive(Debug, Error)]
pub enum CostError {
    #[error("tenant {0} not configured (no row in tenant_llm_budget)")]
    TenantNotConfigured(String),

    #[error(
        "tenant {tenant_id} budget exhausted (cap=${cap_usd:.4}, spent=${spent_usd:.4}, projected=${projected_usd:.4})"
    )]
    BudgetExhausted {
        tenant_id: String,
        cap_usd: f64,
        spent_usd: f64,
        projected_usd: f64,
    },

    #[error("postgres error: {0}")]
    Pg(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct CostGovernor {
    pool: Arc<PgPool>,
}

impl CostGovernor {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    /// Pre-flight check : returns `BudgetState` if call can proceed,
    /// or `BudgetExhausted` if not (and `fail_open=false`).
    #[tracing::instrument(skip(self), fields(tenant_id = %tenant_id))]
    pub async fn check_budget(
        &self,
        tenant_id: &str,
        estimated_cost_usd: f64,
    ) -> Result<BudgetState, CostError> {
        let mut tx = self.pool.begin().await?;

        let row: Option<(Decimal, Decimal, i32, bool)> = sqlx::query_as(
            r#"SELECT monthly_cap_usd, spent_this_month,
                      soft_threshold_pct, fail_open
               FROM tenant_llm_budget
               WHERE tenant_id = $1
               FOR UPDATE"#,
        )
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?;

        let (cap_dec, spent_dec, soft_pct, fail_open) =
            row.ok_or_else(|| CostError::TenantNotConfigured(tenant_id.into()))?;

        let cap = cap_dec.to_f64().unwrap_or(0.0);
        let spent = spent_dec.to_f64().unwrap_or(0.0);
        let projected = spent + estimated_cost_usd;
        let pct_used = if cap > 0.0 {
            (projected / cap) * 100.0
        } else {
            f64::INFINITY
        };
        let soft_warning = pct_used >= soft_pct as f64;

        let state = BudgetState {
            cap_usd: cap,
            spent_usd: spent,
            projected_usd: projected,
            pct_used,
            soft_warning,
            fail_open,
        };

        if projected > cap && !fail_open {
            tx.commit().await?;
            return Err(CostError::BudgetExhausted {
                tenant_id: tenant_id.into(),
                cap_usd: cap,
                spent_usd: spent,
                projected_usd: projected,
            });
        }

        tx.commit().await?;
        Ok(state)
    }

    /// Commit actual cost post-call. Best-effort idempotent.
    #[tracing::instrument(skip(self), fields(tenant_id = %tenant_id, cost_usd = %actual_cost_usd))]
    pub async fn commit_actual_cost(
        &self,
        tenant_id: &str,
        actual_cost_usd: f64,
    ) -> Result<(), CostError> {
        let amount = Decimal::from_str(&format!("{:.6}", actual_cost_usd))
            .unwrap_or_default();
        sqlx::query(
            r#"UPDATE tenant_llm_budget
               SET spent_this_month = spent_this_month + $1,
                   updated_at = now()
               WHERE tenant_id = $2"#,
        )
        .bind(amount)
        .bind(tenant_id)
        .execute(&*self.pool)
        .await?;
        Ok(())
    }

    /// Monthly rollover : reset spent_this_month to 0 for tenants whose
    /// `rollover_dom` matches today's day of month, and last_rollover
    /// is more than 25 days ago (idempotency guard).
    #[tracing::instrument(skip(self))]
    pub async fn rollover_expired_tenants(&self) -> Result<u64, CostError> {
        let n = sqlx::query(
            r#"UPDATE tenant_llm_budget
               SET spent_this_month = 0,
                   last_rollover = CURRENT_DATE,
                   updated_at = now()
               WHERE EXTRACT(DAY FROM CURRENT_DATE)::int = rollover_dom
                 AND last_rollover < CURRENT_DATE - INTERVAL '25 days'"#,
        )
        .execute(&*self.pool)
        .await?
        .rows_affected();
        Ok(n)
    }

    /// Read-only budget state for `efe_get_llm_budget` mcp tool.
    #[tracing::instrument(skip(self), fields(tenant_id = %tenant_id))]
    pub async fn get_budget(
        &self,
        tenant_id: &str,
    ) -> Result<BudgetState, CostError> {
        let row: Option<(Decimal, Decimal, i32, bool)> = sqlx::query_as(
            r#"SELECT monthly_cap_usd, spent_this_month,
                      soft_threshold_pct, fail_open
               FROM tenant_llm_budget
               WHERE tenant_id = $1"#,
        )
        .bind(tenant_id)
        .fetch_optional(&*self.pool)
        .await?;
        let (cap_dec, spent_dec, soft_pct, fail_open) =
            row.ok_or_else(|| CostError::TenantNotConfigured(tenant_id.into()))?;
        let cap = cap_dec.to_f64().unwrap_or(0.0);
        let spent = spent_dec.to_f64().unwrap_or(0.0);
        let pct_used = if cap > 0.0 { (spent / cap) * 100.0 } else { 0.0 };
        Ok(BudgetState {
            cap_usd: cap,
            spent_usd: spent,
            projected_usd: spent,
            pct_used,
            soft_warning: pct_used >= soft_pct as f64,
            fail_open,
        })
    }

    /// Set monthly cap.
    #[tracing::instrument(skip(self))]
    pub async fn set_budget(
        &self,
        tenant_id: &str,
        monthly_cap_usd: f64,
        soft_threshold_pct: i32,
        fail_open: bool,
        rollover_dom: i32,
    ) -> Result<(), CostError> {
        let cap = Decimal::from_str(&format!("{:.4}", monthly_cap_usd))
            .unwrap_or_default();
        sqlx::query(
            r#"INSERT INTO tenant_llm_budget
                (tenant_id, monthly_cap_usd, soft_threshold_pct, fail_open, rollover_dom)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (tenant_id) DO UPDATE
               SET monthly_cap_usd = EXCLUDED.monthly_cap_usd,
                   soft_threshold_pct = EXCLUDED.soft_threshold_pct,
                   fail_open = EXCLUDED.fail_open,
                   rollover_dom = EXCLUDED.rollover_dom,
                   updated_at = now()"#,
        )
        .bind(tenant_id)
        .bind(cap)
        .bind(soft_threshold_pct)
        .bind(fail_open)
        .bind(rollover_dom)
        .execute(&*self.pool)
        .await?;
        Ok(())
    }
}
