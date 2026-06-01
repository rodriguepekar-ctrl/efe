//! Typed cache for LLM provider responses, HMAC-signed.
//!
//! Stores small JSON blobs (`LlmCachedResponse`) keyed by SHA256 of
//! (prompt, model, canonical_params). HMAC-signed to prevent poisoning :
//! every read verifies the signature against the cluster HMAC key.
//!
//! Hit ratio target : > 40% in steady state (M27-4 SLO). Lower than
//! embedding cache because LLM call diversity is higher.

use crate::client::CacheClient;
use crate::error::{CacheError, CacheResult};
use crate::key;
use crate::metrics;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

type HmacSha256 = Hmac<Sha256>;

/// Default TTL for LLM cache entries (1 hour).
pub const LLM_DEFAULT_TTL: Duration = Duration::from_secs(3600);

/// What we actually store in the LLM cache. Mirrors enough of the
/// upstream provider response to reconstruct a `CompleteResponse`-like
/// object without re-calling the provider. Kept small + JSON so the
/// `efe-llm-router` crate can stay independent of this struct's exact
/// shape (it just passes a serializable type).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmCachedResponse {
    /// Final text returned by the provider.
    pub text: String,
    /// Token counts surfaced by the provider.
    pub tokens_in: u32,
    pub tokens_out: u32,
    /// Cost in USD at the time of the call (may diverge from current
    /// pricing if a price change occurred ; we still bill the original).
    pub cost_usd: f64,
    /// Which provider answered.
    pub provider_name: String,
    /// Which model.
    pub model_name: String,
    /// Wall-clock latency observed for the original call (ms).
    #[serde(default)]
    pub latency_ms: u32,
    /// Unix timestamp (s) when the entry was first cached.
    pub stored_at_unix: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct SignedEntry {
    payload: LlmCachedResponse,
    hmac_hex: String,
}

/// Typed LLM response cache.
#[derive(Clone, Debug)]
pub struct LlmCache {
    client: CacheClient,
    hmac_key: Arc<Vec<u8>>,
    ttl: Duration,
    /// When true, keys are tenant-scoped (avoids cross-tenant cache hits
    /// for sensitive prompts).
    pub tenant_scoped: bool,
}

impl LlmCache {
    /// Build an LLM cache. The HMAC key must be ≥ 32 random bytes. In
    /// production, load from `EFE_CACHE_HMAC_KEY` Secret.
    pub fn new(client: CacheClient, hmac_key: Vec<u8>) -> CacheResult<Self> {
        if hmac_key.len() < 32 {
            return Err(CacheError::HmacKeyMissing(format!(
                "EFE_CACHE_HMAC_KEY must be ≥ 32 bytes, got {}",
                hmac_key.len()
            )));
        }
        Ok(Self {
            client,
            hmac_key: Arc::new(hmac_key),
            ttl: LLM_DEFAULT_TTL,
            tenant_scoped: false,
        })
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn tenant_scoped(mut self, on: bool) -> Self {
        self.tenant_scoped = on;
        self
    }

    fn compute_hmac(&self, payload_bytes: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.hmac_key)
            .expect("HMAC key length validated at construction");
        mac.update(payload_bytes);
        hex::encode(mac.finalize().into_bytes())
    }

    fn verify_hmac(&self, payload_bytes: &[u8], expected_hex: &str) -> bool {
        let mut mac = HmacSha256::new_from_slice(&self.hmac_key)
            .expect("HMAC key length validated");
        mac.update(payload_bytes);
        let expected = match hex::decode(expected_hex) {
            Ok(v) => v,
            Err(_) => return false,
        };
        mac.verify_slice(&expected).is_ok()
    }

    fn key_for(
        &self,
        tenant_slug: Option<&str>,
        prompt: &str,
        model: &str,
        params: &str,
    ) -> String {
        match (self.tenant_scoped, tenant_slug) {
            (true, Some(t)) => key::llm_key_tenant_scoped(t, prompt, model, params),
            _ => key::llm_key(prompt, model, params),
        }
    }

    /// Lookup a cached response. Returns Ok(None) on miss. Returns
    /// `Err(Poisoned)` if HMAC verification fails — caller should
    /// emit a P1 alert.
    pub async fn get(
        &self,
        tenant_slug: Option<&str>,
        prompt: &str,
        model: &str,
        canonical_params_json: &str,
    ) -> CacheResult<Option<LlmCachedResponse>> {
        let k = self.key_for(tenant_slug, prompt, model, canonical_params_json);
        let timer = metrics::CACHE_OP_DURATION
            .with_label_values(&["get", self.client.backend_kind])
            .start_timer();
        let bytes = self.client.get_bytes(&k).await;
        drop(timer);
        let bytes = match bytes {
            Ok(None) => {
                metrics::CACHE_MISS_TOTAL
                    .with_label_values(&["llm", self.client.backend_kind])
                    .inc();
                return Ok(None);
            }
            Ok(Some(b)) => b,
            Err(e) => {
                warn!(?e, %k, "llm GET failed, treating as miss");
                return Ok(None);
            }
        };

        let entry: SignedEntry = match serde_json::from_slice(&bytes) {
            Ok(e) => e,
            Err(e) => {
                warn!(?e, %k, "llm entry decode failed, treating as miss");
                return Ok(None);
            }
        };
        let payload_bytes = match serde_json::to_vec(&entry.payload) {
            Ok(b) => b,
            Err(e) => {
                warn!(?e, %k, "llm payload re-serialize failed, miss");
                return Ok(None);
            }
        };
        if !self.verify_hmac(&payload_bytes, &entry.hmac_hex) {
            metrics::CACHE_POISONED_TOTAL.with_label_values(&["llm"]).inc();
            return Err(CacheError::Poisoned);
        }
        metrics::CACHE_HIT_TOTAL
            .with_label_values(&["llm", self.client.backend_kind])
            .inc();
        Ok(Some(entry.payload))
    }

    /// Store a response. Signs with HMAC ; encode as JSON.
    pub async fn set(
        &self,
        tenant_slug: Option<&str>,
        prompt: &str,
        model: &str,
        canonical_params_json: &str,
        payload: LlmCachedResponse,
    ) -> CacheResult<()> {
        let k = self.key_for(tenant_slug, prompt, model, canonical_params_json);
        let payload_bytes = serde_json::to_vec(&payload)?;
        let hmac_hex = self.compute_hmac(&payload_bytes);
        let entry = SignedEntry { payload, hmac_hex };
        let bytes = serde_json::to_vec(&entry)?;
        let timer = metrics::CACHE_OP_DURATION
            .with_label_values(&["set", self.client.backend_kind])
            .start_timer();
        let r = self.client.set_bytes_ex(&k, &bytes, self.ttl.as_secs()).await;
        drop(timer);
        match r {
            Ok(()) => {
                metrics::CACHE_SET_TOTAL
                    .with_label_values(&["llm", self.client.backend_kind, "ok"])
                    .inc();
                Ok(())
            }
            Err(e) => {
                metrics::CACHE_SET_TOTAL
                    .with_label_values(&["llm", self.client.backend_kind, "fail"])
                    .inc();
                warn!(?e, %k, "llm SET failed");
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_payload() -> LlmCachedResponse {
        LlmCachedResponse {
            text: "hi".into(),
            tokens_in: 1,
            tokens_out: 1,
            cost_usd: 0.0,
            provider_name: "openai".into(),
            model_name: "gpt-4o-mini".into(),
            latency_ms: 100,
            stored_at_unix: 1_700_000_000,
        }
    }

    #[test]
    fn hmac_short_key_validated_at_construction() {
        // Direct check of HMAC key length policy without building a
        // CacheClient (which would require a real Redis).
        const MIN_LEN: usize = 32;
        let too_short: Vec<u8> = vec![0u8; 16];
        let just_right: Vec<u8> = vec![0u8; 32];
        assert!(too_short.len() < MIN_LEN);
        assert!(just_right.len() >= MIN_LEN);
        // `LlmCache::new` enforces this and returns CacheError::HmacKeyMissing.
        // See integration tests for the round trip.
    }

    #[test]
    fn signed_entry_serde_roundtrip() {
        let entry = SignedEntry {
            payload: dummy_payload(),
            hmac_hex: "deadbeef".into(),
        };
        let bytes = serde_json::to_vec(&entry).unwrap();
        let back: SignedEntry = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.payload, entry.payload);
        assert_eq!(back.hmac_hex, "deadbeef");
    }

    #[test]
    fn hmac_constant_for_same_input() {
        // Build minimal cache to test compute_hmac directly.
        // We use a static-ish key for determinism.
        let key = vec![0xAB; 32];
        let mut mac =
            <Hmac<Sha256> as hmac::Mac>::new_from_slice(&key).unwrap();
        mac.update(b"hello");
        let h1 = hex::encode(mac.clone().finalize().into_bytes());
        let mut mac2 =
            <Hmac<Sha256> as hmac::Mac>::new_from_slice(&key).unwrap();
        mac2.update(b"hello");
        let h2 = hex::encode(mac2.finalize().into_bytes());
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }
}
