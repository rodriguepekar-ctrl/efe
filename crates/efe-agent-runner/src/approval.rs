//! In-band human approval (Art. 14 oversight).
//!
//! Before any *write* tool call, the agent issues an
//! `ApprovalRequest` ; the run blocks (status `AWAITING_APPROVAL`)
//! until an authorised operator approves or rejects via the gRPC
//! service or portal-admin. On approval we mint a single-use,
//! short-lived (≤ 24 h) `approval_token` (HMAC-MAC over run_id +
//! step + nonce). The agent then presents the token on the next
//! tool call ; the mcp-server validates and **consumes** it.
//!
//! See architecture doc § 17 (approval workflow) for the full state
//! machine.

use crate::{AgentRunnerError, AgentRunnerResult, TenantId};
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "agent_approval_status", rename_all = "SCREAMING_SNAKE_CASE")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApprovalStatus {
    Pending,
    Granted,
    Rejected,
    Expired,
    Consumed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: Uuid,
    pub agent_run_id: Uuid,
    pub tenant_id: TenantId,
    pub step_index: i32,
    pub tool_name: String,
    pub tool_args_json: serde_json::Value,
    pub justification: Option<String>,
    pub requested_at: DateTime<Utc>,
    pub status: ApprovalStatus,
    pub decided_at: Option<DateTime<Utc>>,
    pub decided_by: Option<String>,
    pub decision_reason: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub token_nonce: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateApprovalInput {
    pub agent_run_id: Uuid,
    pub tenant_id: TenantId,
    pub step_index: i32,
    pub tool_name: String,
    pub tool_args_json: serde_json::Value,
    pub justification: Option<String>,
    pub ttl_seconds: i64,
}

pub async fn create(
    tx: &mut Transaction<'_, Postgres>,
    input: &CreateApprovalInput,
) -> AgentRunnerResult<ApprovalRequest> {
    if input.ttl_seconds < 60 || input.ttl_seconds > 86400 {
        return Err(AgentRunnerError::InvalidInput(
            "approval TTL must be 60..=86400 seconds (1 min .. 24 h)".into(),
        ));
    }
    let id = Uuid::new_v4();
    let now = Utc::now();
    let expires_at = now + Duration::seconds(input.ttl_seconds);
    sqlx::query(
        r#"INSERT INTO agent_approval_request
           (id, agent_run_id, tenant_id, step_index, tool_name,
            tool_args_json, justification, requested_at, status, expires_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'PENDING'::agent_approval_status, $9)"#,
    )
    .bind(id)
    .bind(input.agent_run_id)
    .bind(&input.tenant_id.0)
    .bind(input.step_index)
    .bind(&input.tool_name)
    .bind(&input.tool_args_json)
    .bind(&input.justification)
    .bind(now)
    .bind(expires_at)
    .execute(&mut **tx)
    .await?;
    Ok(ApprovalRequest {
        id,
        agent_run_id: input.agent_run_id,
        tenant_id: input.tenant_id.clone(),
        step_index: input.step_index,
        tool_name: input.tool_name.clone(),
        tool_args_json: input.tool_args_json.clone(),
        justification: input.justification.clone(),
        requested_at: now,
        status: ApprovalStatus::Pending,
        decided_at: None,
        decided_by: None,
        decision_reason: None,
        expires_at,
        token_nonce: None,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalToken {
    pub approval_id: Uuid,
    pub agent_run_id: Uuid,
    pub step_index: i32,
    pub nonce: String,
    pub mac_hex: String,
}

pub async fn grant(
    tx: &mut Transaction<'_, Postgres>,
    approval_id: Uuid,
    decided_by: &str,
    reason: Option<&str>,
) -> AgentRunnerResult<ApprovalToken> {
    // Pull row first to make freshness check explicit (the SQL trigger
    // checks too, but a clear error here gives better UX).
    let row: Option<ApprovalRow> = sqlx::query_as(
        r#"SELECT id, agent_run_id, tenant_id, step_index, tool_name,
                  tool_args_json, justification, requested_at, status,
                  decided_at, decided_by, decision_reason, expires_at, token_nonce
           FROM agent_approval_request WHERE id = $1
           FOR UPDATE"#,
    )
    .bind(approval_id)
    .fetch_optional(&mut **tx)
    .await?;
    let row = row.ok_or(AgentRunnerError::ApprovalNotFound(approval_id))?;
    if row.status != ApprovalStatus::Pending {
        return Err(AgentRunnerError::InvalidTransition {
            from: format!("{:?}", row.status),
            to: "GRANTED".into(),
        });
    }
    if row.expires_at <= Utc::now() {
        return Err(AgentRunnerError::ApprovalTokenExpired);
    }

    let nonce = nonce_hex();
    let mac = compute_token_mac(&row.tenant_id, row.agent_run_id, row.step_index, &nonce)?;

    let now = Utc::now();
    sqlx::query(
        r#"UPDATE agent_approval_request
           SET status = 'GRANTED'::agent_approval_status,
               decided_at = $1,
               decided_by = $2,
               decision_reason = $3,
               token_nonce = $4
           WHERE id = $5"#,
    )
    .bind(now)
    .bind(decided_by)
    .bind(reason)
    .bind(&nonce)
    .bind(approval_id)
    .execute(&mut **tx)
    .await?;

    crate::metrics::AGENT_APPROVAL_RESPONSE_LATENCY
        .with_label_values(&[&row.tenant_id, "granted"])
        .observe((now - row.requested_at).num_seconds() as f64);

    Ok(ApprovalToken {
        approval_id: row.id,
        agent_run_id: row.agent_run_id,
        step_index: row.step_index,
        nonce,
        mac_hex: mac,
    })
}

pub async fn reject(
    tx: &mut Transaction<'_, Postgres>,
    approval_id: Uuid,
    decided_by: &str,
    reason: &str,
) -> AgentRunnerResult<()> {
    if reason.trim().len() < 10 {
        return Err(AgentRunnerError::InvalidInput(
            "rejection reason must be ≥ 10 chars".into(),
        ));
    }
    let updated = sqlx::query(
        r#"UPDATE agent_approval_request
           SET status = 'REJECTED'::agent_approval_status,
               decided_at = NOW(),
               decided_by = $1,
               decision_reason = $2
           WHERE id = $3 AND status = 'PENDING'::agent_approval_status"#,
    )
    .bind(decided_by)
    .bind(reason)
    .bind(approval_id)
    .execute(&mut **tx)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(AgentRunnerError::ApprovalNotFound(approval_id));
    }
    Ok(())
}

/// Validate + atomically consume a token (single-use). Returns the
/// consumed `ApprovalRequest`.
pub async fn consume(
    tx: &mut Transaction<'_, Postgres>,
    token: &ApprovalToken,
) -> AgentRunnerResult<ApprovalRequest> {
    let row: Option<ApprovalRow> = sqlx::query_as(
        r#"SELECT id, agent_run_id, tenant_id, step_index, tool_name,
                  tool_args_json, justification, requested_at, status,
                  decided_at, decided_by, decision_reason, expires_at, token_nonce
           FROM agent_approval_request WHERE id = $1
           FOR UPDATE"#,
    )
    .bind(token.approval_id)
    .fetch_optional(&mut **tx)
    .await?;
    let row = row.ok_or(AgentRunnerError::InvalidApprovalToken)?;

    if row.status != ApprovalStatus::Granted {
        return Err(AgentRunnerError::InvalidApprovalToken);
    }
    if row.expires_at <= Utc::now() {
        return Err(AgentRunnerError::ApprovalTokenExpired);
    }
    let nonce = row
        .token_nonce
        .as_deref()
        .ok_or(AgentRunnerError::InvalidApprovalToken)?;
    if nonce != token.nonce {
        return Err(AgentRunnerError::InvalidApprovalToken);
    }
    let expected = compute_token_mac(&row.tenant_id, row.agent_run_id, row.step_index, nonce)?;
    if !constant_time_eq(expected.as_bytes(), token.mac_hex.as_bytes()) {
        return Err(AgentRunnerError::InvalidApprovalToken);
    }

    sqlx::query(
        r#"UPDATE agent_approval_request
           SET status = 'CONSUMED'::agent_approval_status,
               consumed_at = NOW()
           WHERE id = $1"#,
    )
    .bind(row.id)
    .execute(&mut **tx)
    .await?;
    Ok(row.into())
}

/// Sweeper run by the controller every minute : flips PENDING → EXPIRED.
pub async fn sweep_expired(pg: &PgPool) -> AgentRunnerResult<u64> {
    let r = sqlx::query(
        r#"UPDATE agent_approval_request
           SET status = 'EXPIRED'::agent_approval_status,
               decided_at = NOW(),
               decided_by = 'system',
               decision_reason = 'TTL elapsed'
           WHERE status = 'PENDING'::agent_approval_status
             AND expires_at <= NOW()"#,
    )
    .execute(pg)
    .await?;
    let n = r.rows_affected();
    if n > 0 {
        // Bump the metric with a global tenant label since we don't
        // have it here ; the controller can re-attribute later.
        crate::metrics::AGENT_APPROVAL_EXPIRED_TOTAL
            .with_label_values(&["_aggregate"])
            .inc_by(n);
    }
    Ok(n)
}

// ── Helpers ────────────────────────────────────────────────────────

fn nonce_hex() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn approval_hmac_key(tenant_id: &str) -> AgentRunnerResult<Vec<u8>> {
    use sha2::Digest;
    let master = std::env::var("EFE_APPROVAL_HMAC_KEY")
        .unwrap_or_else(|_| "dev-only-not-for-prod-use-32bytes!!".to_string());
    let mut h = Sha256::new();
    h.update(master.as_bytes());
    h.update(b"|m25-approval|");
    h.update(tenant_id.as_bytes());
    Ok(h.finalize().to_vec())
}

fn compute_token_mac(
    tenant_id: &str,
    agent_run_id: Uuid,
    step_index: i32,
    nonce: &str,
) -> AgentRunnerResult<String> {
    let key = approval_hmac_key(tenant_id)?;
    let mut mac = HmacSha256::new_from_slice(&key)
        .map_err(|e| AgentRunnerError::Internal(format!("hmac : {}", e)))?;
    mac.update(agent_run_id.as_bytes());
    mac.update(&step_index.to_be_bytes());
    mac.update(nonce.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[derive(sqlx::FromRow, Clone)]
struct ApprovalRow {
    id: Uuid,
    agent_run_id: Uuid,
    tenant_id: String,
    step_index: i32,
    tool_name: String,
    tool_args_json: serde_json::Value,
    justification: Option<String>,
    requested_at: DateTime<Utc>,
    status: ApprovalStatus,
    decided_at: Option<DateTime<Utc>>,
    decided_by: Option<String>,
    decision_reason: Option<String>,
    expires_at: DateTime<Utc>,
    token_nonce: Option<String>,
}

impl From<ApprovalRow> for ApprovalRequest {
    fn from(r: ApprovalRow) -> Self {
        Self {
            id: r.id,
            agent_run_id: r.agent_run_id,
            tenant_id: TenantId(r.tenant_id),
            step_index: r.step_index,
            tool_name: r.tool_name,
            tool_args_json: r.tool_args_json,
            justification: r.justification,
            requested_at: r.requested_at,
            status: r.status,
            decided_at: r.decided_at,
            decided_by: r.decided_by,
            decision_reason: r.decision_reason,
            expires_at: r.expires_at,
            token_nonce: r.token_nonce,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_key_per_tenant_distinct() {
        let k1 = approval_hmac_key("a").unwrap();
        let k2 = approval_hmac_key("b").unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn token_mac_deterministic() {
        let nonce = "0123456789abcdef".to_string();
        let run = Uuid::new_v4();
        let m1 = compute_token_mac("t1", run, 5, &nonce).unwrap();
        let m2 = compute_token_mac("t1", run, 5, &nonce).unwrap();
        assert_eq!(m1, m2);
        assert_eq!(m1.len(), 64); // 32 bytes hex
    }

    #[test]
    fn token_mac_changes_with_step() {
        let nonce = "0123456789abcdef".to_string();
        let run = Uuid::new_v4();
        let m1 = compute_token_mac("t1", run, 5, &nonce).unwrap();
        let m2 = compute_token_mac("t1", run, 6, &nonce).unwrap();
        assert_ne!(m1, m2);
    }

    #[test]
    fn token_mac_changes_with_tenant() {
        let nonce = "0123456789abcdef".to_string();
        let run = Uuid::new_v4();
        let m1 = compute_token_mac("t1", run, 5, &nonce).unwrap();
        let m2 = compute_token_mac("t2", run, 5, &nonce).unwrap();
        assert_ne!(m1, m2);
    }

    #[test]
    fn ct_eq_works() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
