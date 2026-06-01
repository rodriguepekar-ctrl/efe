//! HMAC-chained audit trail for the active learning loop.
//!
//! Mirrors the M19/M23 pattern : every event (sample enqueued, LLM
//! labeling, human review, retune, promote, GDPR) appends a chained
//! row. Per-tenant chain ; verifiable end-to-end.

use crate::{ActiveLearningError, ActiveLearningResult, TenantId};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRow {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub parent_id: Option<Uuid>,
    pub queue_id: Option<Uuid>,
    pub proposal_id: Option<Uuid>,
    pub retune_run_id: Option<Uuid>,
    pub taxonomy_version_id: Option<Uuid>,
    pub event_type: String,
    pub actor_id: String,
    pub actor_role: Option<String>,
    pub payload_json: serde_json::Value,
    pub payload_hash: String,
    pub hmac: String,
    pub created_at: DateTime<Utc>,
}

#[allow(clippy::too_many_arguments)]
pub async fn append_learning_audit(
    tx: &mut Transaction<'_, Postgres>,
    tenant: &TenantId,
    queue_id: Option<Uuid>,
    proposal_id: Option<Uuid>,
    retune_run_id: Option<Uuid>,
    taxonomy_version_id: Option<Uuid>,
    event_type: &str,
    actor_id: &str,
    actor_role: Option<&str>,
    payload: serde_json::Value,
) -> ActiveLearningResult<Uuid> {
    let canonical = canonical_json(&payload);
    let payload_hash = hex::encode(Sha256::digest(canonical.as_bytes()));

    let parent_id: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM learning_audit
           WHERE tenant_id = $1
           ORDER BY created_at DESC
           LIMIT 1"#,
    )
    .bind(&tenant.0)
    .fetch_optional(&mut **tx).await?;

    let key = audit_hmac_key(&tenant.0)?;
    let mut mac = HmacSha256::new_from_slice(&key)
        .map_err(|e| ActiveLearningError::Internal(format!("HMAC init : {}", e)))?;
    if let Some(p) = parent_id { mac.update(p.as_bytes()); }
    else { mac.update(b"genesis"); }
    mac.update(payload_hash.as_bytes());
    let hmac_hex = hex::encode(mac.finalize().into_bytes());

    let id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO learning_audit
           (id, tenant_id, parent_id, queue_id, proposal_id, retune_run_id,
            taxonomy_version_id, event_type, actor_id, actor_role,
            payload_json, payload_hash, hmac)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)"#,
    )
    .bind(id).bind(&tenant.0).bind(parent_id)
    .bind(queue_id).bind(proposal_id).bind(retune_run_id)
    .bind(taxonomy_version_id)
    .bind(event_type).bind(actor_id).bind(actor_role)
    .bind(&payload).bind(&payload_hash).bind(&hmac_hex)
    .execute(&mut **tx).await?;

    Ok(id)
}

/// Verify chain integrity for a tenant. Returns the row count if OK.
pub async fn verify_chain(
    pg: &PgPool, tenant: &TenantId,
) -> ActiveLearningResult<u64> {
    let rows: Vec<AuditRowPg> = sqlx::query_as(
        r#"SELECT id, tenant_id, parent_id, queue_id, proposal_id,
                  retune_run_id, taxonomy_version_id, event_type,
                  actor_id, actor_role, payload_json, payload_hash, hmac, created_at
           FROM learning_audit
           WHERE tenant_id = $1
           ORDER BY created_at ASC"#,
    )
    .bind(&tenant.0)
    .fetch_all(pg).await?;

    let key = audit_hmac_key(&tenant.0)?;
    let mut prev_id: Option<Uuid> = None;
    let mut count = 0u64;
    for row in &rows {
        if row.parent_id != prev_id {
            return Err(ActiveLearningError::AuditChainBroken {
                parent: row.parent_id.unwrap_or_default(),
            });
        }
        let mut mac = HmacSha256::new_from_slice(&key)
            .map_err(|e| ActiveLearningError::Internal(format!("HMAC init : {}", e)))?;
        match row.parent_id {
            Some(p) => mac.update(p.as_bytes()),
            None => mac.update(b"genesis"),
        }
        mac.update(row.payload_hash.as_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());
        if expected != row.hmac {
            return Err(ActiveLearningError::AuditHmacFailure(row.id));
        }
        let recanon = canonical_json(&row.payload_json);
        let recomputed = hex::encode(Sha256::digest(recanon.as_bytes()));
        if recomputed != row.payload_hash {
            return Err(ActiveLearningError::AuditHmacFailure(row.id));
        }
        prev_id = Some(row.id);
        count += 1;
    }
    Ok(count)
}

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

fn audit_hmac_key(tenant_id: &str) -> ActiveLearningResult<Vec<u8>> {
    let master = std::env::var("EFE_AUDIT_HMAC_KEY")
        .unwrap_or_else(|_| "dev-only-not-for-prod-use-32bytes!!".to_string());
    let mut hasher = Sha256::new();
    hasher.update(master.as_bytes());
    hasher.update(b"|tenant|");
    hasher.update(tenant_id.as_bytes());
    Ok(hasher.finalize().to_vec())
}

#[derive(sqlx::FromRow)]
struct AuditRowPg {
    id: Uuid,
    #[allow(dead_code)] tenant_id: String,
    parent_id: Option<Uuid>,
    #[allow(dead_code)] queue_id: Option<Uuid>,
    #[allow(dead_code)] proposal_id: Option<Uuid>,
    #[allow(dead_code)] retune_run_id: Option<Uuid>,
    #[allow(dead_code)] taxonomy_version_id: Option<Uuid>,
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
    fn canonical_json_sorts() {
        let a = serde_json::json!({"b": 1, "a": 2, "c": [3, 1, 2]});
        let b = serde_json::json!({"a": 2, "b": 1, "c": [3, 1, 2]});
        assert_eq!(canonical_json(&a), canonical_json(&b));
    }

    #[test]
    fn key_per_tenant_distinct() {
        let k1 = audit_hmac_key("acme").unwrap();
        let k2 = audit_hmac_key("beta").unwrap();
        assert_ne!(k1, k2);
        assert_eq!(k1.len(), 32);
    }

    #[test]
    fn payload_hash_deterministic() {
        let p1 = serde_json::json!({"b": 2, "a": 1});
        let p2 = serde_json::json!({"a": 1, "b": 2});
        assert_eq!(canonical_json(&p1), canonical_json(&p2));
    }
}
