//! M31 — Handler state trait for `/v1/usage/*`.

use async_trait::async_trait;
use efe_billing::{BudgetAlert, PriceCents};
use efe_metering::{Meter, RollupRow};

use super::errors::UsageApiError;

/// Snapshot of real-time usage for the last 60 minutes.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UsageNow {
    /// Tenant identifier.
    pub tenant_id: String,
    /// Time window start (ISO-8601 UTC).
    pub since: String,
    /// Time window end (ISO-8601 UTC).
    pub until: String,
    /// Per-meter aggregate count.
    pub by_meter: std::collections::HashMap<Meter, u64>,
}

/// Monthly cost projection.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UsageProjection {
    /// Tenant.
    pub tenant_id: String,
    /// Days elapsed in the current month.
    pub days_elapsed: u32,
    /// Days remaining.
    pub days_remaining: u32,
    /// Cost incurred so far (cents).
    pub current_cost_cents: PriceCents,
    /// Linear projection to month-end (cents).
    pub projected_month_end_cents: PriceCents,
    /// Plan currently in effect.
    pub plan_name: String,
}

/// Handler state trait.
#[async_trait]
pub trait UsageApiState: Clone + Send + Sync + 'static {
    /// Authenticate a request.
    fn auth_tenant_from_request(&self, parts: &axum::http::request::Parts) -> Option<String>;

    /// Read live counters from Redis for the last 60 min.
    async fn usage_now(&self, tenant: &str) -> Result<UsageNow, UsageApiError>;

    /// Read daily rollups for the current month.
    async fn usage_daily(
        &self,
        tenant: &str,
        month_start: &str,
        month_end: &str,
    ) -> Result<Vec<RollupRow>, UsageApiError>;

    /// Compute the cost projection.
    async fn projection(&self, tenant: &str) -> Result<UsageProjection, UsageApiError>;

    /// List budget alerts for a tenant.
    async fn list_budgets(&self, tenant: &str) -> Result<Vec<BudgetAlert>, UsageApiError>;

    /// Create a budget alert.
    async fn create_budget(
        &self,
        tenant: &str,
        alert: BudgetAlert,
    ) -> Result<BudgetAlert, UsageApiError>;

    /// Update a budget alert.
    async fn update_budget(
        &self,
        tenant: &str,
        id: &str,
        alert: BudgetAlert,
    ) -> Result<BudgetAlert, UsageApiError>;

    /// Delete a budget alert.
    async fn delete_budget(&self, tenant: &str, id: &str) -> Result<(), UsageApiError>;
}
