//! Typed cache for embedder outputs.
//!
//! Stores `Vec<f32>` embeddings under SHA256 keys of (text, embedder_version).
//! Serialized via bincode for compactness + speed (no schema drift across
//! versions because we version the embedder explicitly in the key).
//!
//! Hit ratio target : > 80% in steady-state production (M27-3 SLO).
//!
//! Tenant isolation : NOT enforced at this layer because embeddings of
//! identical text + embedder version are mathematically identical
//! regardless of tenant. A collision is a feature, not a leak.

use crate::client::CacheClient;
use crate::error::CacheResult;
use crate::key;
use crate::metrics;
use std::time::Duration;
use tracing::warn;

/// Default TTL for embedding entries (7 days).
pub const EMBEDDING_DEFAULT_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

/// Typed embedding cache facade.
#[derive(Clone, Debug)]
pub struct EmbeddingCache {
    client: CacheClient,
    embedder_version: String,
    ttl: Duration,
}

impl EmbeddingCache {
    /// Build an embedding cache pointing at the given backend, scoped to
    /// the named embedder version. Calling code with a different
    /// embedder must build a separate `EmbeddingCache`.
    pub fn new(client: CacheClient, embedder_version: impl Into<String>) -> Self {
        Self {
            client,
            embedder_version: embedder_version.into(),
            ttl: EMBEDDING_DEFAULT_TTL,
        }
    }

    /// Override the default 7-day TTL.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Lookup an embedding by canonical text. Returns `Ok(None)` on miss
    /// (callers should compute + `set`).
    pub async fn get(&self, text: &str) -> CacheResult<Option<Vec<f32>>> {
        let k = key::embed_key(text, &self.embedder_version);
        let timer = metrics::CACHE_OP_DURATION
            .with_label_values(&["get", self.client.backend_kind])
            .start_timer();
        let r = self.client.get_bytes(&k).await;
        drop(timer);
        match r {
            Ok(None) => {
                metrics::CACHE_MISS_TOTAL
                    .with_label_values(&["embed", self.client.backend_kind])
                    .inc();
                Ok(None)
            }
            Ok(Some(bytes)) => match bincode::deserialize::<Vec<f32>>(&bytes) {
                Ok(v) => {
                    metrics::CACHE_HIT_TOTAL
                        .with_label_values(&["embed", self.client.backend_kind])
                        .inc();
                    Ok(Some(v))
                }
                Err(e) => {
                    warn!(?e, %k, "embedding decode failed, treating as miss");
                    Ok(None)
                }
            },
            Err(e) => {
                warn!(?e, %k, "embedding GET failed, returning miss");
                Ok(None) // fail-open : caller recomputes
            }
        }
    }

    /// Store an embedding. Errors are logged + swallowed (failure-to-cache
    /// must NEVER break the caller path).
    pub async fn set(&self, text: &str, embedding: &[f32]) -> CacheResult<()> {
        let k = key::embed_key(text, &self.embedder_version);
        let bytes = bincode::serialize(embedding).map_err(|e| {
            crate::error::CacheError::Encode(format!("bincode serialize: {}", e))
        })?;
        let ttl_s = self.ttl.as_secs();
        let timer = metrics::CACHE_OP_DURATION
            .with_label_values(&["set", self.client.backend_kind])
            .start_timer();
        let r = self.client.set_bytes_ex(&k, &bytes, ttl_s).await;
        drop(timer);
        match r {
            Ok(()) => {
                metrics::CACHE_SET_TOTAL
                    .with_label_values(&["embed", self.client.backend_kind, "ok"])
                    .inc();
                Ok(())
            }
            Err(e) => {
                metrics::CACHE_SET_TOTAL
                    .with_label_values(&["embed", self.client.backend_kind, "fail"])
                    .inc();
                warn!(?e, %k, "embedding SET failed");
                Err(e)
            }
        }
    }

    /// Get-or-compute helper. Best ergonomics for callers : pass a closure
    /// that computes the embedding on miss, function caches and returns.
    pub async fn get_or_set<F, Fut>(
        &self,
        text: &str,
        compute: F,
    ) -> CacheResult<Vec<f32>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Vec<f32>>,
    {
        if let Ok(Some(v)) = self.get(text).await {
            return Ok(v);
        }
        let v = compute().await;
        let _ = self.set(text, &v).await;
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ttl_is_seven_days() {
        assert_eq!(EMBEDDING_DEFAULT_TTL.as_secs(), 7 * 24 * 3600);
    }

    #[test]
    fn bincode_roundtrip_f32_vec() {
        let v: Vec<f32> = vec![0.1, 0.2, 0.3, -0.4];
        let bytes = bincode::serialize(&v).unwrap();
        let back: Vec<f32> = bincode::deserialize(&bytes).unwrap();
        assert_eq!(v, back);
    }
    // Real Redis-backed tests are integration tests gated by the
    // EFE_REDIS_TEST_URL env var (see tests/integration.rs).
}
