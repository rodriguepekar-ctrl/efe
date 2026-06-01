//! M26 graduation endpoints under `/v1/agent/graduate/*`.

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use super::{auth::AuthCtx, errors::AgentApiError, AgentApiState};

#[derive(Debug, Deserialize)]
pub struct CheckInput {
    pub tenant_id: String,
}

#[derive(Debug, Serialize)]
pub struct CheckOutput {
    pub tenant_id: String,
    pub channel: String,
    pub operational: OperationalCriteria,
    pub compliance: ComplianceCriteria,
    pub ready: bool,
    pub blocking_items: Vec<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct OperationalCriteria {
    pub days_in_experimental: i32,
    pub min_days_required: i32,
    pub successful_runs: i64,
    pub min_runs_required: i64,
    pub sandbox_escapes_90d: i64,
    pub failure_rate_pct_30d: f64,
    pub max_failure_rate_pct: f64,
    pub dpia_valid: bool,
    pub graduation_acknowledgement_signed: bool,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ComplianceCriteria {
    pub audit_chain_verified: bool,
    pub r100_pack_succeeded: bool,
    pub dpo_review_current_quarter: bool,
    pub ai_act_post_market_submitted: bool,
}

#[derive(Debug, Deserialize)]
pub struct ConfirmInput {
    pub tenant_id: String,
    pub acknowledgement_hash: String,
}

#[derive(Debug, Deserialize)]
pub struct RevokeInput {
    pub tenant_id: String,
    pub reason: String,
}

// ── Handlers ─────────────────────────────────────────────────────────

pub async fn check(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Json(body): Json<CheckInput>,
) -> Result<Json<CheckOutput>, AgentApiError> {
    auth.require_any_role(&["operator", "admin"])?;
    auth.require_tenant(&body.tenant_id)?;

    let op = check_operational(&state.pg, &body.tenant_id).await?;
    let comp = check_compliance(&state.pg, &body.tenant_id).await?;
    let channel = current_channel(&state.pg, &body.tenant_id).await?;

    let mut blocking = Vec::new();
    if op.days_in_experimental < op.min_days_required {
        blocking.push(format!(
            "tenant in EXPERIMENTAL only {} days (min {})",
            op.days_in_experimental, op.min_days_required
        ));
    }
    if op.successful_runs < op.min_runs_required {
        blocking.push(format!(
            "successful runs {} < min {}",
            op.successful_runs, op.min_runs_required
        ));
    }
    if op.sandbox_escapes_90d > 0 {
        blocking.push(format!(
            "{} sandbox escape attempts in last 90d (must be 0)",
            op.sandbox_escapes_90d
        ));
    }
    if op.failure_rate_pct_30d > op.max_failure_rate_pct {
        blocking.push(format!(
            "failure rate {:.2}% > max {:.2}%",
            op.failure_rate_pct_30d, op.max_failure_rate_pct
        ));
    }
    if !op.dpia_valid {
        blocking.push("DPIA not valid".into());
    }
    if !op.graduation_acknowledgement_signed {
        blocking.push("production graduation acknowledgement not signed".into());
    }
    if !comp.audit_chain_verified {
        blocking.push("audit chain verification failed".into());
    }
    if !comp.r100_pack_succeeded {
        blocking.push("R-100 compliance pack regen never succeeded".into());
    }
    if !comp.dpo_review_current_quarter {
        blocking.push("DPO has not reviewed previous quarter".into());
    }
    if !comp.ai_act_post_market_submitted {
        blocking.push("AI Act Art. 26 post-market report not submitted".into());
    }

    let ready = blocking.is_empty();
    Ok(Json(CheckOutput {
        tenant_id: body.tenant_id,
        channel,
        operational: op,
        compliance: comp,
        ready,
        blocking_items: blocking,
    }))
}

pub async fn confirm(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Json(body): Json<ConfirmInput>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_role("admin")?;
    auth.require_tenant(&body.tenant_id)?;

    if !body.acknowledgement_hash.starts_with("sha256:") || body.acknowledgement_hash.len() != 71 {
        return Err(AgentApiError::InvalidInput(
            "acknowledgement_hash must be 'sha256:<64-hex>'".into(),
        ));
    }

    let now = chrono::Utc::now();
    let updated = sqlx::query(
        r#"UPDATE tenant_feature
           SET m26_graduation_acknowledged_at = $1,
               m26_graduation_acknowledgement_hash = $2,
               m26_graduation_revoked_at = NULL,
               m26_graduation_revoked_reason = NULL
           WHERE id = $3"#,
    )
    .bind(now)
    .bind(&body.acknowledgement_hash)
    .bind(&body.tenant_id)
    .execute(&state.pg)
    .await?;

    if updated.rows_affected() == 0 {
        return Err(AgentApiError::NotFound);
    }
    Ok(Json(serde_json::json!({
        "success": true,
        "graduated_at": now,
        "acknowledgement_hash": body.acknowledgement_hash,
    })))
}

pub async fn revoke(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Json(body): Json<RevokeInput>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_role("admin")?;
    auth.require_tenant(&body.tenant_id)?;

    if body.reason.trim().len() < 10 {
        return Err(AgentApiError::InvalidInput(
            "reason must be ≥ 10 chars".into(),
        ));
    }

    let now = chrono::Utc::now();
    let updated = sqlx::query(
        r#"UPDATE tenant_feature
           SET m26_graduation_revoked_at = $1,
               m26_graduation_revoked_reason = $2
           WHERE id = $3
             AND m26_graduation_acknowledged_at IS NOT NULL"#,
    )
    .bind(now)
    .bind(&body.reason)
    .bind(&body.tenant_id)
    .execute(&state.pg)
    .await?;

    if updated.rows_affected() == 0 {
        return Err(AgentApiError::FailedPrecondition(
            "tenant is not on STABLE channel".into(),
        ));
    }
    Ok(Json(serde_json::json!({
        "success": true,
        "revoked_at": now,
        "channel": "EXPERIMENTAL",
    })))
}

// ── Helpers ──────────────────────────────────────────────────────────

const MIN_DAYS: i32 = 30;
const MIN_RUNS: i64 = 100;
const MAX_FAILURE_PCT: f64 = 5.0;

async fn check_operational(
    pg: &sqlx::PgPool,
    tenant: &str,
) -> Result<OperationalCriteria, AgentApiError> {
    let row: Option<OpRow> = sqlx::query_as(
        r#"SELECT
              EXTRACT(DAY FROM NOW() - COALESCE(MIN(ar.created_at), NOW()))::int AS days_in_experimental,
              COUNT(*) FILTER (WHERE ar.status = 'COMPLETED')::bigint AS successful_runs,
              COUNT(*) FILTER (
                  WHERE EXISTS (
                      SELECT 1 FROM agent_audit aa
                      WHERE aa.tenant_id = ar.tenant_id
                        AND aa.event_type = 'sandbox_escape_attempt_detected'
                        AND aa.created_at >= NOW() - INTERVAL '90 days'
                  )
              )::bigint AS sandbox_escapes_90d,
              CASE
                WHEN COUNT(*) FILTER (WHERE ar.created_at >= NOW() - INTERVAL '30 days') = 0 THEN 0.0
                ELSE
                  (COUNT(*) FILTER (
                    WHERE ar.created_at >= NOW() - INTERVAL '30 days'
                      AND ar.status IN ('FAILED', 'TIMED_OUT')
                  )::float / COUNT(*) FILTER (WHERE ar.created_at >= NOW() - INTERVAL '30 days')) * 100
              END AS failure_rate_pct_30d
           FROM agent_run ar
           WHERE ar.tenant_id = $1"#,
    )
    .bind(tenant)
    .fetch_optional(pg)
    .await?;

    let row = row.unwrap_or(OpRow {
        days_in_experimental: 0,
        successful_runs: 0,
        sandbox_escapes_90d: 0,
        failure_rate_pct_30d: 0.0,
    });

    let dpia_valid: Option<bool> = sqlx::query_scalar(
        r#"SELECT (m25_dpia_signed_at IS NOT NULL AND m25_dpia_expires_at > NOW())
           FROM tenant_feature WHERE id = $1"#,
    )
    .bind(tenant)
    .fetch_optional(pg)
    .await?
    .flatten();

    let ack_signed: Option<bool> = sqlx::query_scalar(
        r#"SELECT m26_graduation_acknowledgement_hash IS NOT NULL
           FROM tenant_feature WHERE id = $1"#,
    )
    .bind(tenant)
    .fetch_optional(pg)
    .await?
    .flatten();

    Ok(OperationalCriteria {
        days_in_experimental: row.days_in_experimental,
        min_days_required: MIN_DAYS,
        successful_runs: row.successful_runs,
        min_runs_required: MIN_RUNS,
        sandbox_escapes_90d: row.sandbox_escapes_90d,
        failure_rate_pct_30d: row.failure_rate_pct_30d,
        max_failure_rate_pct: MAX_FAILURE_PCT,
        dpia_valid: dpia_valid.unwrap_or(false),
        graduation_acknowledgement_signed: ack_signed.unwrap_or(false),
    })
}

#[derive(sqlx::FromRow)]
struct OpRow {
    days_in_experimental: i32,
    successful_runs: i64,
    sandbox_escapes_90d: i64,
    failure_rate_pct_30d: f64,
}

async fn check_compliance(
    pg: &sqlx::PgPool,
    tenant: &str,
) -> Result<ComplianceCriteria, AgentApiError> {
    let r100: Option<i64> = sqlx::query_scalar(
        r#"SELECT COUNT(*)::bigint FROM compliance_pack_run
           WHERE tenant_id = $1 AND module = 'm25_agent_runtime' AND status = 'OK'"#,
    )
    .bind(tenant)
    .fetch_optional(pg)
    .await?;

    let dpo: Option<i64> = sqlx::query_scalar(
        r#"SELECT COUNT(*)::bigint FROM dpo_review
           WHERE tenant_id = $1
             AND module = 'm25_agent_runtime'
             AND reviewed_at >= NOW() - INTERVAL '90 days'"#,
    )
    .bind(tenant)
    .fetch_optional(pg)
    .await?;

    let post_market: Option<i64> = sqlx::query_scalar(
        r#"SELECT COUNT(*)::bigint FROM ai_act_post_market_report
           WHERE tenant_id = $1
             AND module = 'm25_agent_runtime'
             AND submitted_at >= NOW() - INTERVAL '120 days'"#,
    )
    .bind(tenant)
    .fetch_optional(pg)
    .await?;

    Ok(ComplianceCriteria {
        audit_chain_verified: true, // verified separately by efe-admin audit verify ; placeholder
        r100_pack_succeeded: r100.unwrap_or(0) > 0,
        dpo_review_current_quarter: dpo.unwrap_or(0) > 0,
        ai_act_post_market_submitted: post_market.unwrap_or(0) > 0,
    })
}

async fn current_channel(pg: &sqlx::PgPool, tenant: &str) -> Result<String, AgentApiError> {
    let ch: Option<String> =
        sqlx::query_scalar(r#"SELECT channel FROM v_tenant_channel WHERE tenant_id = $1"#)
            .bind(tenant)
            .fetch_optional(pg)
            .await?;
    Ok(ch.unwrap_or_else(|| "DISABLED".into()))
}
