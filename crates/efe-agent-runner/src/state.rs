//! Encrypted checkpoint state for pause/resume.
//!
//! When the controller (or operator) pauses an agent run, agent-core
//! serializes its in-memory state (LLM message history, tool result
//! cache, planner state) to JSON, encrypts it via per-tenant KMS BYOK
//! (`EFE_KMS_KEY_PER_TENANT_<slug>`), and we persist it in `agent_state`.
//!
//! Resume reads the row, decrypts, hands it back to the new agent-core
//! process via vsock.

use crate::{AgentRunnerError, AgentRunnerResult, TenantId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub agent_run_id: Uuid,
    pub checkpoint_seq: i32,
    pub state_blob_b64: String,    // base64 of KMS-encrypted bytes
    pub kms_key_id: String,
    pub created_at: DateTime<Utc>,
    pub bytes_size: i32,
    pub digest_sha256: String,
}

#[derive(Debug, Clone)]
pub struct CheckpointInput {
    pub agent_run_id: Uuid,
    pub tenant_id: TenantId,
    pub plaintext_blob: Vec<u8>,    // already-serialized agent-core state
    pub kms_key_id: String,
}

/// Persist a new checkpoint. The plaintext blob is encrypted by KMS
/// stub here ; in production this would call the configured KMS. The
/// stub uses an XOR-with-derived-key just so the flow is testable.
pub async fn append_checkpoint(
    tx: &mut Transaction<'_, Postgres>,
    input: &CheckpointInput,
) -> AgentRunnerResult<AgentState> {
    let prior_seq: Option<i32> = sqlx::query_scalar(
        r#"SELECT MAX(checkpoint_seq) FROM agent_state WHERE agent_run_id = $1"#,
    )
    .bind(input.agent_run_id)
    .fetch_optional(&mut **tx)
    .await?
    .flatten();
    let seq = prior_seq.unwrap_or(0) + 1;

    let cipher = stub_encrypt(&input.plaintext_blob, &input.kms_key_id, &input.tenant_id);
    let bytes_size = cipher.len() as i32;
    let digest = sha2_hex(&input.plaintext_blob);
    let blob_b64 = base64_encode(&cipher);
    let now = Utc::now();

    sqlx::query(
        r#"INSERT INTO agent_state
           (agent_run_id, checkpoint_seq, state_blob_b64, kms_key_id,
            bytes_size, digest_sha256, created_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
    )
    .bind(input.agent_run_id)
    .bind(seq)
    .bind(&blob_b64)
    .bind(&input.kms_key_id)
    .bind(bytes_size)
    .bind(&digest)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    Ok(AgentState {
        agent_run_id: input.agent_run_id,
        checkpoint_seq: seq,
        state_blob_b64: blob_b64,
        kms_key_id: input.kms_key_id.clone(),
        created_at: now,
        bytes_size,
        digest_sha256: digest,
    })
}

/// Fetch the latest checkpoint for resume. Returns plaintext after
/// decryption.
pub async fn fetch_latest(
    pg: &PgPool,
    tenant: &TenantId,
    agent_run_id: Uuid,
) -> AgentRunnerResult<(AgentState, Vec<u8>)> {
    let row: Option<StateRow> = sqlx::query_as(
        r#"SELECT agent_run_id, checkpoint_seq, state_blob_b64, kms_key_id,
                  bytes_size, digest_sha256, created_at
           FROM agent_state
           WHERE agent_run_id = $1
           ORDER BY checkpoint_seq DESC LIMIT 1"#,
    )
    .bind(agent_run_id)
    .fetch_optional(pg)
    .await?;
    let row = row.ok_or(AgentRunnerError::RunNotFound(agent_run_id))?;

    let cipher = base64_decode(&row.state_blob_b64)
        .map_err(|e| AgentRunnerError::Internal(format!("base64 decode : {}", e)))?;
    let plain = stub_decrypt(&cipher, &row.kms_key_id, tenant);
    let actual_digest = sha2_hex(&plain);
    if actual_digest != row.digest_sha256 {
        return Err(AgentRunnerError::Internal(
            "checkpoint digest mismatch — possible tampering".into(),
        ));
    }
    Ok((row.into(), plain))
}

#[derive(sqlx::FromRow)]
struct StateRow {
    agent_run_id: Uuid,
    checkpoint_seq: i32,
    state_blob_b64: String,
    kms_key_id: String,
    bytes_size: i32,
    digest_sha256: String,
    created_at: DateTime<Utc>,
}

impl From<StateRow> for AgentState {
    fn from(r: StateRow) -> Self {
        Self {
            agent_run_id: r.agent_run_id,
            checkpoint_seq: r.checkpoint_seq,
            state_blob_b64: r.state_blob_b64,
            kms_key_id: r.kms_key_id,
            created_at: r.created_at,
            bytes_size: r.bytes_size,
            digest_sha256: r.digest_sha256,
        }
    }
}

// ── Stub crypto (replaced in production by KMS calls) ──────────────

fn stub_key(kms_key_id: &str, tenant: &TenantId) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"efe-m25-kms-stub|");
    h.update(kms_key_id.as_bytes());
    h.update(b"|");
    h.update(tenant.0.as_bytes());
    h.finalize().to_vec()
}

fn stub_encrypt(plaintext: &[u8], kms_key_id: &str, tenant: &TenantId) -> Vec<u8> {
    let key = stub_key(kms_key_id, tenant);
    plaintext
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}

fn stub_decrypt(ciphertext: &[u8], kms_key_id: &str, tenant: &TenantId) -> Vec<u8> {
    // XOR is symmetric.
    stub_encrypt(ciphertext, kms_key_id, tenant)
}

fn sha2_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn base64_decode(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_crypto_roundtrip() {
        let t = TenantId("acme".into());
        let plain = b"agent-state-blob-123";
        let cipher = stub_encrypt(plain, "k1", &t);
        assert_ne!(&cipher[..], &plain[..]);
        let back = stub_decrypt(&cipher, "k1", &t);
        assert_eq!(back, plain);
    }

    #[test]
    fn stub_crypto_per_tenant_distinct() {
        let plain = b"x";
        let c1 = stub_encrypt(plain, "k1", &TenantId("a".into()));
        let c2 = stub_encrypt(plain, "k1", &TenantId("b".into()));
        assert_ne!(c1, c2);
    }

    #[test]
    fn sha2_hex_known_vector() {
        // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        assert_eq!(
            sha2_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
