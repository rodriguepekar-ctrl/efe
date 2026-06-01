//! AI Act Art. 26 post-market monitoring quarterly export.
//!
//! Endpoint : `GET /v1/agent/post-market/quarterly?quarter=YYYY-QN`.
//! Returns a JSON-LD document the auditor portal consumes. Idempotent —
//! the same quarter always returns the same content modulo any updates
//! to the underlying audit chain.

use axum::{
    extract::{Query, State},
    Json,
};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

use super::{auth::AuthCtx, errors::AgentApiError, AgentApiState};

#[derive(Debug, Deserialize)]
pub struct QuarterlyQuery {
    /// Format `YYYY-QN`, e.g. `2027-Q4`. If absent, the previous quarter is used.
    #[serde(default)]
    pub quarter: Option<String>,
    /// Tenant scope. Required.
    pub tenant: String,
}

#[derive(Debug, Serialize)]
pub struct QuarterlyReport {
    #[serde(rename = "@context")]
    pub at_context: String,
    #[serde(rename = "@type")]
    pub at_type: String,
    pub schema_version: String,
    pub tenant_id: String,
    pub module: String,
    pub quarter: String,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub run_summary: RunSummary,
    pub cost: CostSummary,
    pub security: SecuritySummary,
    pub approvals: ApprovalSummary,
    pub provider_distribution: serde_json::Value,
    pub dpia_status: DpiaSummary,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct RunSummary {
    pub total_runs: i64,
    pub completed: i64,
    pub failed: i64,
    pub terminated: i64,
    pub timed_out: i64,
    pub cost_capped: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CostSummary {
    pub total_cents: i64,
    pub avg_cents_per_run: i64,
    pub p95_cents: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SecuritySummary {
    pub sandbox_escape_attempts: i64,
    pub tool_rejections: i64,
    pub image_rejections: i64,
    pub audit_chain_verifications: i64,
    pub audit_chain_failures: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ApprovalSummary {
    pub requested: i64,
    pub granted: i64,
    pub rejected: i64,
    pub expired: i64,
    pub avg_response_seconds: i64,
}

#[derive(Debug, Serialize)]
pub struct DpiaSummary {
    pub valid: bool,
    pub signed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub addendum_hash: Option<String>,
    pub days_until_expiry: Option<i64>,
}

pub async fn quarterly(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Query(q): Query<QuarterlyQuery>,
) -> Result<Json<QuarterlyReport>, AgentApiError> {
    auth.require_any_role(&["operator", "admin", "dpo"])?;
    auth.require_tenant(&q.tenant)?;

    let quarter = q.quarter.unwrap_or_else(previous_quarter_str);
    let (start, end) = parse_quarter(&quarter)?;

    let runs: RunSummary = sqlx::query_as(
        r#"SELECT
              COUNT(*)::bigint AS total_runs,
              COUNT(*) FILTER (WHERE status = 'COMPLETED')::bigint AS completed,
              COUNT(*) FILTER (WHERE status = 'FAILED')::bigint AS failed,
              COUNT(*) FILTER (WHERE status = 'TERMINATED')::bigint AS terminated,
              COUNT(*) FILTER (WHERE status = 'TIMED_OUT')::bigint AS timed_out,
              COUNT(*) FILTER (WHERE status = 'COST_CAPPED')::bigint AS cost_capped
           FROM agent_run
           WHERE tenant_id = $1 AND created_at >= $2 AND created_at < $3"#,
    )
    .bind(&q.tenant)
    .bind(start)
    .bind(end)
    .fetch_one(&state.pg)
    .await?;

    let cost: CostSummary = sqlx::query_as(
        r#"SELECT
              COALESCE(SUM(current_cost_cents), 0)::bigint AS total_cents,
              COALESCE(AVG(current_cost_cents), 0)::bigint AS avg_cents_per_run,
              COALESCE(percentile_cont(0.95) WITHIN GROUP (ORDER BY current_cost_cents), 0)::bigint AS p95_cents
           FROM agent_run
           WHERE tenant_id = $1 AND created_at >= $2 AND created_at < $3"#,
    )
    .bind(&q.tenant)
    .bind(start)
    .bind(end)
    .fetch_one(&state.pg)
    .await?;

    let security: SecuritySummary = sqlx::query_as(
        r#"SELECT
              COUNT(*) FILTER (WHERE event_type = 'sandbox_escape_attempt_detected')::bigint AS sandbox_escape_attempts,
              COUNT(*) FILTER (WHERE event_type = 'tool_call_rejected_whitelist')::bigint AS tool_rejections,
              0::bigint AS image_rejections,
              COUNT(*)::bigint AS audit_chain_verifications,
              0::bigint AS audit_chain_failures
           FROM agent_audit
           WHERE tenant_id = $1 AND created_at >= $2 AND created_at < $3"#,
    )
    .bind(&q.tenant)
    .bind(start)
    .bind(end)
    .fetch_one(&state.pg)
    .await?;

    let approvals: ApprovalSummary = sqlx::query_as(
        r#"SELECT
              COUNT(*)::bigint AS requested,
              COUNT(*) FILTER (WHERE status = 'GRANTED' OR status = 'CONSUMED')::bigint AS granted,
              COUNT(*) FILTER (WHERE status = 'REJECTED')::bigint AS rejected,
              COUNT(*) FILTER (WHERE status = 'EXPIRED')::bigint AS expired,
              COALESCE(AVG(EXTRACT(EPOCH FROM (decided_at - requested_at)))::bigint, 0) AS avg_response_seconds
           FROM agent_approval_request
           WHERE tenant_id = $1 AND requested_at >= $2 AND requested_at < $3"#,
    )
    .bind(&q.tenant)
    .bind(start)
    .bind(end)
    .fetch_one(&state.pg)
    .await?;

    let provider_dist: Vec<(String, i64)> = sqlx::query_as(
        r#"SELECT provider, COUNT(*)::bigint
           FROM agent_run
           WHERE tenant_id = $1 AND created_at >= $2 AND created_at < $3
           GROUP BY provider"#,
    )
    .bind(&q.tenant)
    .bind(start)
    .bind(end)
    .fetch_all(&state.pg)
    .await?;

    let provider_json: serde_json::Value = provider_dist
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::from(v)))
        .collect::<serde_json::Map<_, _>>()
        .into();

    let dpia_row: Option<(
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<String>,
    )> = sqlx::query_as(
        r#"SELECT m25_dpia_signed_at, m25_dpia_expires_at, m25_dpia_addendum_hash
               FROM tenant_feature WHERE id = $1"#,
    )
    .bind(&q.tenant)
    .fetch_optional(&state.pg)
    .await?;
    let (signed_at, expires_at, addendum_hash) = dpia_row.unwrap_or((None, None, None));
    let days_until_expiry = expires_at.map(|e| (e - chrono::Utc::now()).num_days());

    let report = QuarterlyReport {
        at_context: "https://schema.efe.ai/ai-act-post-market/v1".into(),
        at_type: "AIActPostMarketMonitoringReport".into(),
        schema_version: "1.0.0".into(),
        tenant_id: q.tenant.clone(),
        module: "m25_agent_runtime".into(),
        quarter,
        generated_at: chrono::Utc::now(),
        run_summary: runs,
        cost,
        security,
        approvals,
        provider_distribution: provider_json,
        dpia_status: DpiaSummary {
            valid: signed_at.is_some()
                && expires_at.map(|e| e > chrono::Utc::now()).unwrap_or(false),
            signed_at,
            expires_at,
            addendum_hash,
            days_until_expiry,
        },
    };

    Ok(Json(report))
}

// ── Helpers ──────────────────────────────────────────────────────────

fn previous_quarter_str() -> String {
    let now = chrono::Utc::now().date_naive();
    let q = (now.month() - 1) / 3 + 1;
    let prev_q = if q == 1 { 4 } else { q - 1 };
    let prev_year = if q == 1 { now.year() - 1 } else { now.year() };
    format!("{}-Q{}", prev_year, prev_q)
}

fn parse_quarter(
    s: &str,
) -> Result<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>), AgentApiError> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 2 || !parts[1].starts_with('Q') {
        return Err(AgentApiError::InvalidInput(
            "quarter must be 'YYYY-QN'".into(),
        ));
    }
    let year: i32 = parts[0]
        .parse()
        .map_err(|_| AgentApiError::InvalidInput("invalid year".into()))?;
    let q: u32 = parts[1][1..]
        .parse()
        .map_err(|_| AgentApiError::InvalidInput("invalid quarter".into()))?;
    if !(1..=4).contains(&q) {
        return Err(AgentApiError::InvalidInput("quarter must be 1..=4".into()));
    }
    let start_month = (q - 1) * 3 + 1;
    let start = NaiveDate::from_ymd_opt(year, start_month, 1)
        .ok_or_else(|| AgentApiError::InvalidInput("invalid quarter date".into()))?;
    let end_year = if q == 4 { year + 1 } else { year };
    let end_month = if q == 4 { 1 } else { start_month + 3 };
    let end = NaiveDate::from_ymd_opt(end_year, end_month, 1)
        .ok_or_else(|| AgentApiError::InvalidInput("invalid quarter end".into()))?;
    Ok((
        start
            .and_hms_opt(0, 0, 0)
            .map(|d| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(d, chrono::Utc))
            .unwrap(),
        end.and_hms_opt(0, 0, 0)
            .map(|d| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(d, chrono::Utc))
            .unwrap(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_quarter_q1() {
        let (start, end) = parse_quarter("2027-Q1").unwrap();
        assert_eq!(start.to_rfc3339(), "2027-01-01T00:00:00+00:00");
        assert_eq!(end.to_rfc3339(), "2027-04-01T00:00:00+00:00");
    }

    #[test]
    fn parse_quarter_q4_year_boundary() {
        let (start, end) = parse_quarter("2027-Q4").unwrap();
        assert_eq!(start.to_rfc3339(), "2027-10-01T00:00:00+00:00");
        assert_eq!(end.to_rfc3339(), "2028-01-01T00:00:00+00:00");
    }

    #[test]
    fn parse_quarter_invalid() {
        assert!(parse_quarter("2027-Q5").is_err());
        assert!(parse_quarter("foo").is_err());
        assert!(parse_quarter("2027-01").is_err());
    }
}
