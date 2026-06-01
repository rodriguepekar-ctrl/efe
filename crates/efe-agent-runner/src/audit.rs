//! HMAC-chained audit trail for agent runtime events.
//!
//! Mirrors the M19/M23/M24 pattern : every lifecycle event (run_created,
//! tool_call_attempted, approval_granted, sandbox_escape_attempt_detected,
//! final_output, run_terminated, …) appends a chained row.
//!
//! **CRITICAL** : per-tenant chain keyed by `EFE_AUDIT_HMAC_KEY` →
//! tampering is detectable end-to-end via `verify_chain`. PagerDuty
//! P0 if any chain breaks. See `docs/runbooks/M25_AGENT_RUNTIME_RUNBOOK.md`
//! § 5 (audit chain HMAC failure).

use crate::{AgentRunnerError, AgentRunnerResult, TenantId};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// Domain-typed audit event kinds (matches CHECK in migration 0016).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAuditEvent {
    RunCreated,
    MicrovmProvisioned,
    RunStarted,
    LlmCallCompleted,
    ToolCallAttempted,
    ToolCallCompleted,
    ToolCallRejectedWhitelist,
    ToolCallRejectedApproval,
    ApprovalRequested,
    ApprovalGranted,
    ApprovalRejected,
    ApprovalExpired,
    ApprovalConsumed,
    PauseRequested,
    RunPaused,
    RunResumed,
    CostWarning80pct,
    CostCapExceeded,
    BudgetExceeded,
    SandboxEscapeAttemptDetected,
    FinalOutput,
    RunCompleted,
    RunTerminated,
    RunFailed,
    RunTimedOut,
    GdprArt17Cascade,
}

impl AgentAuditEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RunCreated => "run_created",
            Self::MicrovmProvisioned => "microvm_provisioned",
            Self::RunStarted => "run_started",
            Self::LlmCallCompleted => "llm_call_completed",
            Self::ToolCallAttempted => "tool_call_attempted",
            Self::ToolCallCompleted => "tool_call_completed",
            Self::ToolCallRejectedWhitelist => "tool_call_rejected_whitelist",
            Self::ToolCallRejectedApproval => "tool_call_rejected_approval",
            Self::ApprovalRequested => "approval_requested",
            Self::ApprovalGranted => "approval_granted",
            Self::ApprovalRejected => "approval_rejected",
            Self::ApprovalExpired => "approval_expired",
            Self::ApprovalConsumed => "approval_consumed",
            Self::PauseRequested => "pause_requested",
            Self::RunPaused => "run_paused",
            Self::RunResumed => "run_resumed",
            Self::CostWarning80pct => "cost_warning_80pct",
            Self::CostCapExceeded => "cost_cap_exceeded",
            Self::BudgetExceeded => "budget_exceeded",
            Self::SandboxEscapeAttemptDetected => "sandbox_escape_attempt_detected",
            Self::FinalOutput => "final_output",
            Self::RunCompleted => "run_completed",
            Self::RunTerminated => "run_terminated",
            Self::RunFailed => "run_failed",
            Self::RunTimedOut => "run_timed_out",
            Self::GdprArt17Cascade => "gdpr_art17_cascade",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAuditRow {
    pub id: Uuid,
    pub tenant_id: String,
    pub parent_id: Option<Uuid>,
    pub agent_run_id: Option<Uuid>,
    pub agent_step_id: Option<Uuid>,
    pub event_type: String,
    pub actor_id: String,
    pub actor_role: Option<String>,
    pub payload_json: serde_json::Value,
    pub payload_hash: String,
    pub hmac: String,
    pub created_at: DateTime<Utc>,
}

/// Append an audit row inside an active transaction (so it stays
/// atomic with the state-changing INSERT/UPDATE).
#[allow(clippy::too_many_arguments)]
pub async fn append_agent_audit(
    tx: &mut Transaction<'_, Postgres>,
    tenant: &TenantId,
    agent_run_id: Option<Uuid>,
    agent_step_id: Option<Uuid>,
    event_type: AgentAuditEvent,
    actor_id: &str,
    actor_role: Option<&str>,
    payload: serde_json::Value,
) -> AgentRunnerResult<Uuid> {
    let canonical = canonical_json(&payload);
    let payload_hash = hex::encode(Sha256::digest(canonical.as_bytes()));

    // Read prior tail (per-tenant chain).
    let parent_id: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM agent_audit
           WHERE tenant_id = $1
           ORDER BY created_at DESC
           LIMIT 1"#,
    )
    .bind(&tenant.0)
    .fetch_optional(&mut **tx)
    .await?;

    let key = audit_hmac_key(&tenant.0)?;
    let mut mac = HmacSha256::new_from_slice(&key)
        .map_err(|e| AgentRunnerError::Internal(format!("HMAC init : {}", e)))?;
    if let Some(p) = parent_id { mac.update(p.as_bytes()); }
    else { mac.update(b"genesis"); }
    mac.update(payload_hash.as_bytes());
    let hmac_hex = hex::encode(mac.finalize().into_bytes());

    let id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO agent_audit
           (id, tenant_id, parent_id, agent_run_id, agent_step_id,
            event_type, actor_id, actor_role,
            payload_json, payload_hash, hmac)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
    )
    .bind(id)
    .bind(&tenant.0)
    .bind(parent_id)
    .bind(agent_run_id)
    .bind(agent_step_id)
    .bind(event_type.as_str())
    .bind(actor_id)
    .bind(actor_role)
    .bind(&payload)
    .bind(&payload_hash)
    .bind(&hmac_hex)
    .execute(&mut **tx)
    .await?;

    Ok(id)
}

/// Convenience for non-transactional callers (e.g. mid-run telemetry
/// from a controller). Opens its own short txn.
#[allow(clippy::too_many_arguments)]
pub async fn append_agent_audit_owned(
    pg: &PgPool,
    tenant: &TenantId,
    agent_run_id: Option<Uuid>,
    agent_step_id: Option<Uuid>,
    event_type: AgentAuditEvent,
    actor_id: &str,
    actor_role: Option<&str>,
    payload: serde_json::Value,
) -> AgentRunnerResult<Uuid> {
    let mut tx = pg.begin().await?;
    let id = append_agent_audit(
        &mut tx, tenant, agent_run_id, agent_step_id,
        event_type, actor_id, actor_role, payload,
    ).await?;
    tx.commit().await?;
    Ok(id)
}

/// Verify the full chain for one tenant. Returns row count if OK.
pub async fn verify_chain(pg: &PgPool, tenant: &TenantId) -> AgentRunnerResult<u64> {
    let rows: Vec<AuditRowPg> = sqlx::query_as(
        r#"SELECT id, tenant_id, parent_id, agent_run_id, agent_step_id,
                  event_type, actor_id, actor_role,
                  payload_json, payload_hash, hmac, created_at
           FROM agent_audit
           WHERE tenant_id = $1
           ORDER BY created_at ASC"#,
    )
    .bind(&tenant.0)
    .fetch_all(pg)
    .await?;

    let key = audit_hmac_key(&tenant.0)?;
    let mut prev_id: Option<Uuid> = None;
    let mut count = 0u64;
    for row in &rows {
        if row.parent_id != prev_id {
            return Err(AgentRunnerError::AuditChainBroken {
                expected_parent: prev_id,
                got_parent: row.parent_id,
                row_id: row.id,
            });
        }
        let mut mac = HmacSha256::new_from_slice(&key)
            .map_err(|e| AgentRunnerError::Internal(format!("HMAC init : {}", e)))?;
        match row.parent_id {
            Some(p) => mac.update(p.as_bytes()),
            None => mac.update(b"genesis"),
        }
        mac.update(row.payload_hash.as_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());
        if expected != row.hmac {
            return Err(AgentRunnerError::AuditHmacFailure(row.id));
        }
        // Re-canonicalize stored payload, recompute payload_hash.
        let recanon = canonical_json(&row.payload_json);
        let recomputed = hex::encode(Sha256::digest(recanon.as_bytes()));
        if recomputed != row.payload_hash {
            return Err(AgentRunnerError::AuditHmacFailure(row.id));
        }
        prev_id = Some(row.id);
        count += 1;
    }
    Ok(count)
}

/// Canonical JSON : recursively sort object keys → deterministic hash.
fn canonical_json(v: &serde_json::Value) -> String {
    fn sort_recursively(v: &serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::Object(m) => {
                let mut sorted = std::collections::BTreeMap::new();
                for (k, val) in m {
                    sorted.insert(k.clone(), sort_recursively(val));
                }
                serde_json::Value::Object(sorted.into_iter().collect())
            }
            serde_json::Value::Array(a) => serde_json::Value::Array(
                a.iter().map(sort_recursively).collect(),
            ),
            _ => v.clone(),
        }
    }
    serde_json::to_string(&sort_recursively(v)).unwrap_or_default()
}

/// Per-tenant HMAC key derived from `EFE_AUDIT_HMAC_KEY` (or dev fallback).
fn audit_hmac_key(tenant_id: &str) -> AgentRunnerResult<Vec<u8>> {
    let master = std::env::var("EFE_AUDIT_HMAC_KEY")
        .unwrap_or_else(|_| "dev-only-not-for-prod-use-32bytes!!".to_string());
    let mut hasher = Sha256::new();
    hasher.update(master.as_bytes());
    hasher.update(b"|tenant|");
    hasher.update(tenant_id.as_bytes());
    hasher.update(b"|m25|");
    Ok(hasher.finalize().to_vec())
}

#[derive(sqlx::FromRow)]
struct AuditRowPg {
    id: Uuid,
    #[allow(dead_code)] tenant_id: String,
    parent_id: Option<Uuid>,
    #[allow(dead_code)] agent_run_id: Option<Uuid>,
    #[allow(dead_code)] agent_step_id: Option<Uuid>,
    #[allow(dead_code)] event_type: String,
    #[allow(dead_code)] actor_id: String,
    #[allow(dead_code)] actor_role: Option<String>,
    payload_json: serde_json::Value,
    payload_hash: String,
    hmac: String,
    #[allow(dead_code)] created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_is_order_independent() {
        let a = serde_json::json!({"b": 1, "a": 2, "c": [3, 1, 2]});
        let b = serde_json::json!({"a": 2, "b": 1, "c": [3, 1, 2]});
        assert_eq!(canonical_json(&a), canonical_json(&b));
    }

    #[test]
    fn audit_key_distinct_per_tenant() {
        let k1 = audit_hmac_key("acme").unwrap();
        let k2 = audit_hmac_key("beta").unwrap();
        assert_ne!(k1, k2);
        assert_eq!(k1.len(), 32);
    }

    #[test]
    fn audit_key_distinct_from_other_milestones() {
        // M25 derives with `|m25|` suffix → different from M24 even
        // for the same tenant. (M24 derived without suffix.)
        let k_m25 = audit_hmac_key("acme").unwrap();
        // Recompute the M24-style key inline.
        let master = std::env::var("EFE_AUDIT_HMAC_KEY")
            .unwrap_or_else(|_| "dev-only-not-for-prod-use-32bytes!!".to_string());
        let mut hasher = Sha256::new();
        hasher.update(master.as_bytes());
        hasher.update(b"|tenant|");
        hasher.update(b"acme");
        let k_m24 = hasher.finalize().to_vec();
        assert_ne!(k_m25, k_m24);
    }

    #[test]
    fn event_str_stable() {
        assert_eq!(AgentAuditEvent::RunCreated.as_str(), "run_created");
        assert_eq!(
            AgentAuditEvent::SandboxEscapeAttemptDetected.as_str(),
            "sandbox_escape_attempt_detected"
        );
        assert_eq!(AgentAuditEvent::GdprArt17Cascade.as_str(), "gdpr_art17_cascade");
    }
}
