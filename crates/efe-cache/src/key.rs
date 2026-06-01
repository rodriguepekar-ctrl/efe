//! Cache key derivation. All keys are namespaced + SHA256-hashed when
//! they could contain user data. Hashing is for :
//! - constant-length keys (Redis Cluster sharding fairness),
//! - PII-safety in Redis logs (no input text leaks),
//! - tenant cross-isolation (when prefixed by tenant slug).
//!
//! Conventions :
//! - `embed:{sha256}` — embedder result cache, tenant-agnostic.
//! - `llm:{sha256}` — LLM response cache, tenant-agnostic by default.
//! - `tenant:{slug}:llm:{sha256}` — when llm cache is tenant-scoped
//!   (e.g. fine-tuned model per tenant).
//! - `quota:tenant:{slug}:{endpoint}:{epoch_unit}` — sliding counter.
//! - `revoked_jwt:{jti}` — JWT revocation list.
//! - `tenant_limits:{slug}` — cached per-tenant limits (TenantQuotaLayer).

use sha2::{Digest, Sha256};

/// Build the embedder cache key.
///
/// Concatenates `text` with `:` and `embedder_version` so swapping the
/// model invalidates every cached entry automatically.
pub fn embed_key(text: &str, embedder_version: &str) -> String {
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    h.update(b":");
    h.update(embedder_version.as_bytes());
    let digest = h.finalize();
    format!("embed:{}", hex::encode(digest))
}

/// Build the LLM-response cache key.
///
/// Inputs : prompt, model, canonical-param-json (JSON with keys sorted).
pub fn llm_key(prompt: &str, model: &str, canonical_params_json: &str) -> String {
    let mut h = Sha256::new();
    h.update(prompt.as_bytes());
    h.update(b":");
    h.update(model.as_bytes());
    h.update(b":");
    h.update(canonical_params_json.as_bytes());
    let digest = h.finalize();
    format!("llm:{}", hex::encode(digest))
}

/// Tenant-scoped LLM key variant (when `tenant_feature.private_llm = true`).
pub fn llm_key_tenant_scoped(
    tenant_slug: &str,
    prompt: &str,
    model: &str,
    canonical_params_json: &str,
) -> String {
    let mut h = Sha256::new();
    h.update(prompt.as_bytes());
    h.update(b":");
    h.update(model.as_bytes());
    h.update(b":");
    h.update(canonical_params_json.as_bytes());
    let digest = h.finalize();
    format!("tenant:{}:llm:{}", tenant_slug, hex::encode(digest))
}

/// Build quota counter key for the current epoch unit.
/// `epoch_unit` = minute (60s) for short windows, day (86400s) for daily totals.
pub fn quota_key(tenant_slug: &str, endpoint: &str, epoch_unit: u64) -> String {
    format!("quota:tenant:{}:{}:{}", tenant_slug, endpoint, epoch_unit)
}

/// JWT revocation list key.
pub fn revoked_jwt_key(jti: &str) -> String {
    format!("revoked_jwt:{}", jti)
}

/// Cached tenant limits (loaded from `tenant_feature` table).
pub fn tenant_limits_key(tenant_slug: &str) -> String {
    format!("tenant_limits:{}", tenant_slug)
}

/// Compute current epoch minute (UTC).
pub fn epoch_minute_now() -> u64 {
    let now = chrono::Utc::now().timestamp() as u64;
    now / 60
}

/// Compute current epoch day (UTC).
pub fn epoch_day_now() -> u64 {
    let now = chrono::Utc::now().timestamp() as u64;
    now / 86_400
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_keys_are_deterministic() {
        let k1 = embed_key("hello", "v1");
        let k2 = embed_key("hello", "v1");
        assert_eq!(k1, k2);
        assert!(k1.starts_with("embed:"));
        assert_eq!(k1.len(), "embed:".len() + 64); // 32-byte sha256 hex
    }

    #[test]
    fn embedder_version_invalidates() {
        let k1 = embed_key("hello", "v1");
        let k2 = embed_key("hello", "v2");
        assert_ne!(k1, k2);
    }

    #[test]
    fn llm_key_distinguishes_params() {
        let k1 = llm_key("foo", "gpt-4", r#"{"temperature":0.7}"#);
        let k2 = llm_key("foo", "gpt-4", r#"{"temperature":0.8}"#);
        assert_ne!(k1, k2);
    }

    #[test]
    fn tenant_scoped_llm_differs_from_shared() {
        let shared = llm_key("foo", "m", "{}");
        let scoped = llm_key_tenant_scoped("acme", "foo", "m", "{}");
        assert_ne!(shared, scoped);
        assert!(scoped.contains("tenant:acme:"));
    }

    #[test]
    fn quota_key_layout() {
        let k = quota_key("acme", "classify", 28_000_000);
        assert_eq!(k, "quota:tenant:acme:classify:28000000");
    }

    #[test]
    fn revoked_key_layout() {
        let k = revoked_jwt_key("550e8400-e29b-41d4-a716-446655440000");
        assert!(k.starts_with("revoked_jwt:"));
    }
}
