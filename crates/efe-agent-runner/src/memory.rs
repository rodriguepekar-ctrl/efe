//! Cross-run memory (opt-in, namespace-scoped).
//!
//! When `tenant_feature.m25_memory_enabled = true`, an agent run can
//! read/write key-value entries scoped to a *namespace* (typically the
//! template slug : `research_lite`) and a *visibility* (`private`,
//! `team`). The agent reads on resume to remember prior facts.
//!
//! **Privacy** : memory is per-tenant, namespace-isolated, and erased
//! when the tenant is offboarded. PII redaction is applied on write
//! (the agent-core PII redactor masks emails, phone numbers, IBANs).

use crate::{AgentRunnerError, AgentRunnerResult, TenantId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// Stored as TEXT in `agent_memory.visibility` (CHECK constraint
/// enforces 'private' / 'team'). We bypass `sqlx::Type` derive (which
/// is awkward for TEXT-backed enums) and round-trip via `&str` /
/// `String`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryVisibility {
    Private,
    Team,
}

impl MemoryVisibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Team => "team",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "private" => Some(Self::Private),
            "team" => Some(Self::Team),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub namespace: String,
    pub created_by_run_id: Option<Uuid>,
    pub visibility: MemoryVisibility,
    pub key: String,
    pub value_json: serde_json::Value,
    pub ttl_expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertMemoryInput {
    pub tenant_id: TenantId,
    pub namespace: String,
    pub created_by_run_id: Option<Uuid>,
    pub visibility: MemoryVisibility,
    pub key: String,
    pub value_json: serde_json::Value,
    pub ttl_seconds: Option<u64>,
}

impl UpsertMemoryInput {
    pub fn validate(&self) -> AgentRunnerResult<()> {
        if self.namespace.is_empty() || self.namespace.len() > 64 {
            return Err(AgentRunnerError::InvalidInput(
                "memory namespace must be 1..=64 chars".into(),
            ));
        }
        if self.key.is_empty() || self.key.len() > 200 {
            return Err(AgentRunnerError::InvalidInput(
                "memory key must be 1..=200 chars".into(),
            ));
        }
        let serialized = serde_json::to_vec(&self.value_json)
            .map_err(|e| AgentRunnerError::Internal(e.to_string()))?;
        if serialized.len() > 32 * 1024 {
            return Err(AgentRunnerError::InvalidInput(format!(
                "memory value too large : {} bytes (max 32 KiB)",
                serialized.len()
            )));
        }
        Ok(())
    }
}

pub async fn upsert(
    tx: &mut Transaction<'_, Postgres>,
    input: &UpsertMemoryInput,
) -> AgentRunnerResult<MemoryEntry> {
    input.validate()?;
    let now = Utc::now();
    let ttl = input
        .ttl_seconds
        .map(|s| now + chrono::Duration::seconds(s as i64));
    let id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO agent_memory
           (id, tenant_id, namespace, created_by_run_id, visibility,
            key, value_json, ttl_expires_at, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
           ON CONFLICT (tenant_id, namespace, visibility, key)
           DO UPDATE SET value_json = EXCLUDED.value_json,
                         ttl_expires_at = EXCLUDED.ttl_expires_at,
                         updated_at = EXCLUDED.updated_at,
                         created_by_run_id = EXCLUDED.created_by_run_id"#,
    )
    .bind(id)
    .bind(&input.tenant_id.0)
    .bind(&input.namespace)
    .bind(input.created_by_run_id)
    .bind(input.visibility.as_str())
    .bind(&input.key)
    .bind(&input.value_json)
    .bind(ttl)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(MemoryEntry {
        id,
        tenant_id: input.tenant_id.clone(),
        namespace: input.namespace.clone(),
        created_by_run_id: input.created_by_run_id,
        visibility: input.visibility,
        key: input.key.clone(),
        value_json: input.value_json.clone(),
        ttl_expires_at: ttl,
        created_at: now,
        updated_at: now,
    })
}

pub async fn lookup(
    pg: &PgPool,
    tenant: &TenantId,
    namespace: &str,
    visibility: MemoryVisibility,
    key: &str,
) -> AgentRunnerResult<Option<MemoryEntry>> {
    let row: Option<MemoryRow> = sqlx::query_as(
        r#"SELECT id, tenant_id, namespace, created_by_run_id, visibility,
                  key, value_json, ttl_expires_at, created_at, updated_at
           FROM agent_memory
           WHERE tenant_id = $1 AND namespace = $2
             AND visibility = $3 AND key = $4
             AND (ttl_expires_at IS NULL OR ttl_expires_at > NOW())"#,
    )
    .bind(&tenant.0)
    .bind(namespace)
    .bind(visibility.as_str())
    .bind(key)
    .fetch_optional(pg)
    .await?;
    Ok(row.map(Into::into))
}

pub async fn list_namespace(
    pg: &PgPool,
    tenant: &TenantId,
    namespace: &str,
) -> AgentRunnerResult<Vec<MemoryEntry>> {
    let rows: Vec<MemoryRow> = sqlx::query_as(
        r#"SELECT id, tenant_id, namespace, created_by_run_id, visibility,
                  key, value_json, ttl_expires_at, created_at, updated_at
           FROM agent_memory
           WHERE tenant_id = $1 AND namespace = $2
             AND (ttl_expires_at IS NULL OR ttl_expires_at > NOW())
           ORDER BY updated_at DESC"#,
    )
    .bind(&tenant.0)
    .bind(namespace)
    .fetch_all(pg)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// GDPR Art. 17 cascade : delete all memory for tenant. Used at offboard.
pub async fn delete_all_for_tenant(
    pg: &PgPool,
    tenant: &TenantId,
) -> AgentRunnerResult<u64> {
    let r = sqlx::query(r#"DELETE FROM agent_memory WHERE tenant_id = $1"#)
        .bind(&tenant.0)
        .execute(pg)
        .await?;
    Ok(r.rows_affected())
}

#[derive(sqlx::FromRow)]
struct MemoryRow {
    id: Uuid,
    tenant_id: String,
    namespace: String,
    created_by_run_id: Option<Uuid>,
    visibility: String,                 // 'private' | 'team' (CHECK enforced in SQL)
    key: String,
    value_json: serde_json::Value,
    ttl_expires_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<MemoryRow> for MemoryEntry {
    fn from(r: MemoryRow) -> Self {
        Self {
            id: r.id,
            tenant_id: TenantId(r.tenant_id),
            namespace: r.namespace,
            created_by_run_id: r.created_by_run_id,
            visibility: MemoryVisibility::from_str(&r.visibility)
                .unwrap_or(MemoryVisibility::Private),
            key: r.key,
            value_json: r.value_json,
            ttl_expires_at: r.ttl_expires_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_input() -> UpsertMemoryInput {
        UpsertMemoryInput {
            tenant_id: TenantId("acme".into()),
            namespace: "research_lite".into(),
            created_by_run_id: Some(Uuid::new_v4()),
            visibility: MemoryVisibility::Private,
            key: "favorite_database".into(),
            value_json: serde_json::json!({"db": "postgres"}),
            ttl_seconds: Some(3600),
        }
    }

    #[test]
    fn validate_ok() {
        assert!(ok_input().validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_namespace() {
        let mut i = ok_input();
        i.namespace = "".into();
        assert!(i.validate().is_err());
    }

    #[test]
    fn validate_rejects_oversized_value() {
        let mut i = ok_input();
        i.value_json = serde_json::Value::String("x".repeat(64 * 1024));
        assert!(i.validate().is_err());
    }
}
