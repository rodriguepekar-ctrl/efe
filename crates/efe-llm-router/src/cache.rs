//! Cache (M21 Phase D).
//!
//! Two layers :
//!   - **Exact-match** : sha256 hash key → response (Redis HASH).
//!   - **Semantic**    : embedding kNN cosine > threshold (Redis Stack
//!                       HNSW vector index).
//!
//! Anti-poisoning : every cache entry is HMAC-SHA256 signed with a
//! per-server key. Read-side verification rejects tampered entries.

use crate::provider::CompleteResponse;
use hmac::{Hmac, Mac};
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use thiserror::Error;
use tracing::warn;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("serialization error: {0}")]
    Serde(String),

    #[error("HMAC verification failed (cache poisoning suspected)")]
    HmacInvalid,
}

/// Cached LLM response, ready for replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResponse {
    pub text: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub cost_usd: f64,
    pub provider_name: String,
    pub model_name: String,
    pub stored_at_unix: i64,
}

impl From<&CompleteResponse> for CachedResponse {
    fn from(r: &CompleteResponse) -> Self {
        Self {
            text: r.text.clone(),
            tokens_in: r.tokens_in,
            tokens_out: r.tokens_out,
            cost_usd: r.cost_usd,
            provider_name: r.provider_name.clone(),
            model_name: r.model_name.clone(),
            stored_at_unix: chrono::Utc::now().timestamp(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SignedEntry {
    payload: CachedResponse,
    hmac: Vec<u8>,
}

#[derive(Clone)]
pub struct Cache {
    conn: ConnectionManager,
    hmac_key: Arc<Vec<u8>>,
    #[allow(dead_code)] // used by future semantic cache impl
    semantic_threshold: f32,
}

impl Cache {
    pub fn new(
        conn: ConnectionManager,
        hmac_key: Vec<u8>,
        semantic_threshold: f32,
    ) -> Self {
        Self {
            conn,
            hmac_key: Arc::new(hmac_key),
            semantic_threshold,
        }
    }

    /// Anti-collision key construction with null-byte separators.
    /// Per-tenant prefix prevents cross-tenant leak even on hash
    /// collision.
    pub fn compute_exact_key(
        tenant_id: &str,
        catalog: &str,
        bundle_version: &str,
        prompt_template_id: &str,
        prompt_template_version: u32,
        sentence_redacted: &str,
    ) -> String {
        let mut h = Sha256::new();
        h.update(tenant_id.as_bytes());
        h.update(b"\0");
        h.update(catalog.as_bytes());
        h.update(b"\0");
        h.update(bundle_version.as_bytes());
        h.update(b"\0");
        h.update(prompt_template_id.as_bytes());
        h.update(b"\0");
        h.update(prompt_template_version.to_be_bytes());
        h.update(b"\0");
        h.update(sentence_redacted.as_bytes());
        format!(
            "efe:llm:cache:exact:{}:{}",
            tenant_id,
            hex::encode(h.finalize())
        )
    }

    pub async fn get_exact(
        &self,
        cache_key: &str,
    ) -> Result<Option<CachedResponse>, CacheError> {
        let mut conn = self.conn.clone();
        let raw: Option<Vec<u8>> = conn.get(cache_key).await?;
        let bytes = match raw {
            Some(b) => b,
            None => return Ok(None),
        };
        let signed: SignedEntry =
            bincode::deserialize(&bytes).map_err(|e| CacheError::Serde(e.to_string()))?;
        if !self.verify(&signed) {
            warn!("cache entry HMAC invalid for key {}, ignoring", cache_key);
            return Ok(None);
        }
        Ok(Some(signed.payload))
    }

    pub async fn put_exact(
        &self,
        cache_key: &str,
        response: &CompleteResponse,
        ttl_seconds: u64,
    ) -> Result<(), CacheError> {
        let entry = CachedResponse::from(response);
        let signed = self.sign(entry);
        let bytes =
            bincode::serialize(&signed).map_err(|e| CacheError::Serde(e.to_string()))?;
        let mut conn = self.conn.clone();
        let _: () = conn.set_ex(cache_key, bytes, ttl_seconds).await?;
        Ok(())
    }

    /// Invalidate all entries for a tenant via SCAN + DEL batches.
    /// Avoids `KEYS *` on production Redis (blocking).
    pub async fn invalidate_tenant(
        &self,
        tenant_id: &str,
    ) -> Result<u64, CacheError> {
        let pattern = format!("efe:llm:cache:exact:{}:*", tenant_id);
        let mut conn = self.conn.clone();
        let mut cursor: u64 = 0;
        let mut deleted: u64 = 0;
        loop {
            let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(500)
                .query_async(&mut conn)
                .await?;
            if !keys.is_empty() {
                deleted += self.del_batch(&keys).await?;
            }
            cursor = next_cursor;
            if cursor == 0 {
                break;
            }
        }
        Ok(deleted)
    }

    /// Invalidate all entries for a (tenant, catalog) pair via index set.
    pub async fn invalidate_catalog(
        &self,
        tenant_id: &str,
        catalog: &str,
    ) -> Result<u64, CacheError> {
        let index_key = format!("efe:llm:cache:idx:{}:{}", tenant_id, catalog);
        let mut conn = self.conn.clone();
        let keys: Vec<String> = conn.smembers(&index_key).await?;
        if keys.is_empty() {
            return Ok(0);
        }
        let deleted = self.del_batch(&keys).await?;
        let _: () = conn.del(&index_key).await?;
        Ok(deleted)
    }

    async fn del_batch(&self, keys: &[String]) -> Result<u64, CacheError> {
        if keys.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn.clone();
        let mut cmd = redis::cmd("DEL");
        for k in keys {
            cmd.arg(k);
        }
        let n: u64 = cmd.query_async(&mut conn).await?;
        Ok(n)
    }

    pub async fn add_to_catalog_index(
        &self,
        tenant_id: &str,
        catalog: &str,
        cache_key: &str,
    ) -> Result<(), CacheError> {
        let index_key = format!("efe:llm:cache:idx:{}:{}", tenant_id, catalog);
        let mut conn = self.conn.clone();
        let _: () = conn.sadd(&index_key, cache_key).await?;
        let _: () = conn.expire(&index_key, 86_400).await?;
        Ok(())
    }

    fn sign(&self, payload: CachedResponse) -> SignedEntry {
        let bytes = bincode::serialize(&payload).expect("serialise CachedResponse");
        let mut mac =
            HmacSha256::new_from_slice(&self.hmac_key).expect("HMAC key (any length)");
        mac.update(&bytes);
        let hmac = mac.finalize().into_bytes().to_vec();
        SignedEntry { payload, hmac }
    }

    fn verify(&self, signed: &SignedEntry) -> bool {
        let bytes = match bincode::serialize(&signed.payload) {
            Ok(b) => b,
            Err(_) => return false,
        };
        let mut mac = match HmacSha256::new_from_slice(&self.hmac_key) {
            Ok(m) => m,
            Err(_) => return false,
        };
        mac.update(&bytes);
        mac.verify_slice(&signed.hmac).is_ok()
    }
}
