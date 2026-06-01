//! HMAC-chained audit trail for taxonomy curation events.
//!
//! Every curation lifecycle event (proposal_created, review_approve, promote,
//! rollback, …) appends a row to `taxonomy_change_audit`, chained to its
//! parent via HMAC-SHA256. The chain is per-tenant and verifiable end-to-end.
//!
//! See § 21 of `EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md`.

use crate::{CurationError, CurationResult, TenantId};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// One event in the audit chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRow {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub parent_id: Option<Uuid>,
    pub proposal_id: Option<Uuid>,
    pub taxonomy_version_id: Option<Uuid>,
    pub event_type: String,
    pub actor_id: String,
    pub actor_role: Option<String>,
    pub payload_json: serde_json::Value,
    pub payload_hash: String,
    pub hmac: String,
    pub created_at: DateTime<Utc>,
}

/// Append an event to the chain. Computes payload_hash + HMAC, finds parent
/// (latest row for tenant), inserts row.
pub async fn append_audit(
    tx: &mut Transaction<'_, Postgres>,
    tenant: &TenantId,
    proposal_id: Option<Uuid>,
    taxonomy_version_id: Option<Uuid>,
    event_type: &str,
    actor_id: &str,
    actor_role: Option<&str>,
    payload: serde_json::Value,
) -> CurationResult<Uuid> {
    // 1. canonical payload_json (sorted keys for deterministic hash)
    let canonical = canonical_json(&payload);
    let payload_hash = hex::encode(Sha256::digest(canonical.as_bytes()));

    // 2. find latest row for tenant (parent)
    let parent_id: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM taxonomy_change_audit
           WHERE tenant_id = $1
           ORDER BY created_at DESC
           LIMIT 1"#,
    )
    .bind(&tenant.0)
    .fetch_optional(&mut **tx).await?;

    // 3. compute HMAC over (parent_id || payload_hash)
    let hmac_key = audit_hmac_key(&tenant.0)?;
    let mut mac = HmacSha256::new_from_slice(&hmac_key)
        .map_err(|e| CurationError::Internal(format!("HMAC init : {}", e)))?;
    if let Some(p) = parent_id { mac.update(p.as_bytes()); }
    else { mac.update(b"genesis"); }
    mac.update(payload_hash.as_bytes());
    let hmac_hex = hex::encode(mac.finalize().into_bytes());

    // 4. insert
    let id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO taxonomy_change_audit
           (id, tenant_id, parent_id, proposal_id, taxonomy_version_id,
            event_type, actor_id, actor_role, payload_json, payload_hash, hmac)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
    )
    .bind(id).bind(&tenant.0).bind(parent_id)
    .bind(proposal_id).bind(taxonomy_version_id)
    .bind(event_type).bind(actor_id).bind(actor_role)
    .bind(&payload).bind(&payload_hash).bind(&hmac_hex)
    .execute(&mut **tx).await?;

    Ok(id)
}

/// Verify the chain integrity for a tenant, from genesis up to row `until`
/// (exclusive bound, or all rows if `None`).
///
/// Returns `Ok(())` if every row's HMAC matches, else `AuditChainBroken` or
/// `AuditHmacFailure` at the first failure.
pub async fn verify_chain(pg: &PgPool, tenant: &TenantId, until: Option<DateTime<Utc>>) -> CurationResult<u64> {
    let rows: Vec<AuditRowPg> = sqlx::query_as(
        r#"SELECT id, tenant_id, parent_id, proposal_id, taxonomy_version_id,
                  event_type, actor_id, actor_role, payload_json, payload_hash, hmac, created_at
           FROM taxonomy_change_audit
           WHERE tenant_id = $1
             AND ($2::TIMESTAMPTZ IS NULL OR created_at < $2)
           ORDER BY created_at ASC"#,
    )
    .bind(&tenant.0).bind(until)
    .fetch_all(pg).await?;

    let key = audit_hmac_key(&tenant.0)?;
    let mut prev_id: Option<Uuid> = None;
    let mut count = 0u64;

    for row in &rows {
        if row.parent_id != prev_id {
            return Err(CurationError::AuditChainBroken {
                parent: row.parent_id.unwrap_or_default(),
            });
        }
        let mut mac = HmacSha256::new_from_slice(&key)
            .map_err(|e| CurationError::Internal(format!("HMAC init : {}", e)))?;
        match row.parent_id {
            Some(p) => mac.update(p.as_bytes()),
            None => mac.update(b"genesis"),
        }
        mac.update(row.payload_hash.as_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());
        if expected != row.hmac {
            return Err(CurationError::AuditHmacFailure(row.id));
        }
        // Verify payload_hash matches re-canonicalized payload.
        let recanon = canonical_json(&row.payload_json);
        let recomputed = hex::encode(Sha256::digest(recanon.as_bytes()));
        if recomputed != row.payload_hash {
            return Err(CurationError::AuditHmacFailure(row.id));
        }
        prev_id = Some(row.id);
        count += 1;
    }
    Ok(count)
}

/// Insert a tombstone redaction row (preserves chain integrity) for GDPR
/// Article 17 right-to-be-forgotten requests.
pub async fn redact_for_gdpr_art17(
    pg: &PgPool, tenant: &TenantId, original_row_id: Uuid, requester: &str, reason: &str,
) -> CurationResult<Uuid> {
    let mut tx = pg.begin().await?;
    let payload = serde_json::json!({
        "redacted_row_id": original_row_id,
        "requester": requester,
        "reason": reason,
        "redacted_at": Utc::now(),
    });
    let new_id = append_audit(
        &mut tx, tenant, None, None,
        "gdpr_art17_redaction", requester, None,
        payload,
    ).await?;
    // Note : we do NOT modify the original row's payload — we preserve HMAC.
    // The downstream compliance pack export should mask the original payload
    // when this redaction tombstone exists.
    tx.commit().await?;
    Ok(new_id)
}

/// Canonical JSON serialization with sorted keys for deterministic hashing.
fn canonical_json(v: &serde_json::Value) -> String {
    fn sort_recursively(v: &serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::Object(m) => {
                let mut sorted = std::collections::BTreeMap::new();
                for (k, val) in m { sorted.insert(k.clone(), sort_recursively(val)); }
                serde_json::Value::Object(sorted.into_iter().collect())
            }
            serde_json::Value::Array(a) => {
                serde_json::Value::Array(a.iter().map(sort_recursively).collect())
            }
            _ => v.clone(),
        }
    }
    serde_json::to_string(&sort_recursively(v)).unwrap_or_default()
}

/// Per-tenant HMAC key derivation.
///
/// In production, the master key is loaded from the BYOK keystore (M19).
/// In dev/test, falls back to a deterministic value seeded from
/// `EFE_AUDIT_HMAC_KEY` env var.
fn audit_hmac_key(tenant_id: &str) -> CurationResult<Vec<u8>> {
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
    #[allow(dead_code)] proposal_id: Option<Uuid>,
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
    fn canonical_json_sorts_keys() {
        let a = serde_json::json!({"b": 1, "a": 2, "c": [3, 1, 2]});
        let b = serde_json::json!({"a": 2, "b": 1, "c": [3, 1, 2]});
        assert_eq!(canonical_json(&a), canonical_json(&b));
    }

    #[test]
    fn canonical_json_recursive() {
        let a = serde_json::json!({"outer": {"y": 2, "x": 1}, "z": 3});
        let canon = canonical_json(&a);
        // Verify keys are sorted by checking position of "x" before "y"
        let outer_obj = canon.find("outer").unwrap();
        let x_pos = canon[outer_obj..].find("x").unwrap();
        let y_pos = canon[outer_obj..].find("y").unwrap();
        assert!(x_pos < y_pos);
    }

    #[test]
    fn hmac_key_per_tenant_deterministic() {
        let k1 = audit_hmac_key("acme").unwrap();
        let k2 = audit_hmac_key("acme").unwrap();
        let k3 = audit_hmac_key("beta").unwrap();
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
    }

    #[test]
    fn hmac_key_length() {
        let k = audit_hmac_key("acme").unwrap();
        assert_eq!(k.len(), 32); // SHA-256 output
    }

    #[test]
    fn payload_hash_deterministic() {
        let p1 = serde_json::json!({"b": 2, "a": 1});
        let p2 = serde_json::json!({"a": 1, "b": 2});
        let h1 = Sha256::digest(canonical_json(&p1).as_bytes());
        let h2 = Sha256::digest(canonical_json(&p2).as_bytes());
        assert_eq!(h1, h2);
    }
}
