// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N17** : Model registry — expected hashes + price
//! table.
//!
//! Architecture § 7.3 + ADR-7.

use std::time::SystemTime;

use async_trait::async_trait;
use dashmap::DashMap;
use efe_nano_sdk::{Blake3Hash, ModelIdentity};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─────────────────────────────────────────────────────────────────────
//  Domain types
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : registered model with expected hash + price.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisteredModel {
    pub identity: ModelIdentity,
    /// Expected content hash.  `None` for hosted models we can't
    /// fingerprint ; pre-call hash verification is skipped for those
    /// unless `dev_skip_hash_check = false` is set on a registered
    /// hash entry.
    pub expected_content_hash: Option<Blake3Hash>,
    pub price_per_million_input_tokens_cents: f64,
    pub price_per_million_output_tokens_cents: f64,
    pub prices_table_version: u32,
    pub registered_at: SystemTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated_at: Option<SystemTime>,
    /// Dev-only escape hatch — skip hash verification.  Production
    /// builds must NOT use this.
    #[serde(default)]
    pub dev_skip_hash_check: bool,
}

impl RegisteredModel {
    /// **N17** : Convenience for hosted models without expected hash.
    pub fn hosted(
        identity: ModelIdentity,
        price_input_cents_per_million: f64,
        price_output_cents_per_million: f64,
        prices_table_version: u32,
    ) -> Self {
        Self {
            identity,
            expected_content_hash: None,
            price_per_million_input_tokens_cents: price_input_cents_per_million,
            price_per_million_output_tokens_cents: price_output_cents_per_million,
            prices_table_version,
            registered_at: SystemTime::now(),
            deprecated_at: None,
            dev_skip_hash_check: false,
        }
    }

    /// **N17** : True if this model is deprecated as of `now`.
    pub fn is_deprecated(&self, now: SystemTime) -> bool {
        self.deprecated_at.is_some_and(|d| d <= now)
    }

    /// **N17** : Compute cost in cents from token counts.
    ///
    /// `cost = (input_tokens / 1_000_000) × input_price + ...`
    /// Returns the deterministic cost capped at `f64::MAX / 2` to
    /// guard against overflow (F11).  Second tuple element is the
    /// `overflow_capped` flag.
    pub fn compute_cost_cents(
        &self,
        input_tokens: u32,
        output_tokens: u32,
    ) -> (f64, bool) {
        const CAP: f64 = f64::MAX / 2.0;
        let input_cost = (f64::from(input_tokens) / 1_000_000.0)
            * self.price_per_million_input_tokens_cents;
        let output_cost = (f64::from(output_tokens) / 1_000_000.0)
            * self.price_per_million_output_tokens_cents;
        let total = input_cost + output_cost;
        if !total.is_finite() || total > CAP {
            (CAP, true)
        } else {
            (total, false)
        }
    }
}

/// **Sprint 9 — N17** : closed taxonomy of registry errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ModelRegistryError {
    /// Identity not present in registry.
    #[error("model not registered : {provider}/{model_id}@{version}")]
    NotRegistered {
        provider: String,
        model_id: String,
        version: String,
    },

    /// Model registered but deprecated.
    #[error("model deprecated : {provider}/{model_id}@{version}")]
    Deprecated {
        provider: String,
        model_id: String,
        version: String,
    },

    /// Pre-call hash verification failed (architecture F3 + ADR-7).
    #[error("model hash mismatch : expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },

    /// Backend I/O fault.
    #[error("registry backend error : {message}")]
    Backend { message: String },
}

// ─────────────────────────────────────────────────────────────────────
//  ModelRegistry trait
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : Model registry.
#[async_trait]
pub trait ModelRegistry: Send + Sync + 'static {
    /// **N17** : Look up a registered model.
    async fn lookup(
        &self,
        identity: &ModelIdentity,
    ) -> Result<RegisteredModel, ModelRegistryError>;

    /// **N17** : Verify the actual content hash matches the expected
    /// hash.  Returns `Ok(())` if the model has no expected hash OR
    /// the model has `dev_skip_hash_check = true`.  Returns
    /// `Err(HashMismatch)` on drift.
    async fn verify_hash(
        &self,
        identity: &ModelIdentity,
        actual_hash: &Blake3Hash,
    ) -> Result<(), ModelRegistryError>;

    /// **N17** : Register a new model (or replace existing
    /// registration).
    async fn register(&self, model: RegisteredModel) -> Result<(), ModelRegistryError>;

    /// **N17** : Mark a model as deprecated.
    async fn deprecate(
        &self,
        identity: &ModelIdentity,
        at: SystemTime,
    ) -> Result<(), ModelRegistryError>;

    /// **N17** : List currently-active (non-deprecated) models.
    async fn list_active(&self) -> Result<Vec<RegisteredModel>, ModelRegistryError>;
}

// ─────────────────────────────────────────────────────────────────────
//  InMemoryModelRegistry
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : in-memory implementation backed by `DashMap`.
pub struct InMemoryModelRegistry {
    models: DashMap<RegistryKey, RegisteredModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RegistryKey {
    provider: String,
    model_id: String,
    version: String,
}

impl RegistryKey {
    fn from_identity(identity: &ModelIdentity) -> Self {
        Self {
            provider: identity.provider.clone(),
            model_id: identity.model_id.clone(),
            version: identity.requested_version.clone(),
        }
    }
}

impl std::fmt::Debug for InMemoryModelRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryModelRegistry")
            .field("models", &self.models.len())
            .finish()
    }
}

impl Default for InMemoryModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryModelRegistry {
    /// **N17** : Construct an empty registry.
    pub fn new() -> Self {
        Self {
            models: DashMap::new(),
        }
    }

    /// **N17** : Number of registered models.
    pub fn len(&self) -> usize {
        self.models.len()
    }

    /// **N17** : True if registry is empty.
    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }
}

#[async_trait]
impl ModelRegistry for InMemoryModelRegistry {
    async fn lookup(
        &self,
        identity: &ModelIdentity,
    ) -> Result<RegisteredModel, ModelRegistryError> {
        let key = RegistryKey::from_identity(identity);
        let entry = self.models.get(&key).ok_or_else(|| {
            ModelRegistryError::NotRegistered {
                provider: key.provider.clone(),
                model_id: key.model_id.clone(),
                version: key.version.clone(),
            }
        })?;
        let model = entry.value().clone();
        if model.is_deprecated(SystemTime::now()) {
            return Err(ModelRegistryError::Deprecated {
                provider: key.provider,
                model_id: key.model_id,
                version: key.version,
            });
        }
        Ok(model)
    }

    async fn verify_hash(
        &self,
        identity: &ModelIdentity,
        actual_hash: &Blake3Hash,
    ) -> Result<(), ModelRegistryError> {
        let model = self.lookup(identity).await?;
        if model.dev_skip_hash_check {
            return Ok(());
        }
        let Some(expected) = model.expected_content_hash else {
            // Hosted model with no expected hash — verification is
            // not applicable.
            return Ok(());
        };
        if &expected == actual_hash {
            Ok(())
        } else {
            Err(ModelRegistryError::HashMismatch {
                expected: expected.to_hex(),
                actual: actual_hash.to_hex(),
            })
        }
    }

    async fn register(&self, model: RegisteredModel) -> Result<(), ModelRegistryError> {
        let key = RegistryKey::from_identity(&model.identity);
        self.models.insert(key, model);
        Ok(())
    }

    async fn deprecate(
        &self,
        identity: &ModelIdentity,
        at: SystemTime,
    ) -> Result<(), ModelRegistryError> {
        let key = RegistryKey::from_identity(identity);
        let mut entry = self.models.get_mut(&key).ok_or_else(|| {
            ModelRegistryError::NotRegistered {
                provider: key.provider.clone(),
                model_id: key.model_id.clone(),
                version: key.version.clone(),
            }
        })?;
        entry.deprecated_at = Some(at);
        Ok(())
    }

    async fn list_active(&self) -> Result<Vec<RegisteredModel>, ModelRegistryError> {
        let now = SystemTime::now();
        Ok(self
            .models
            .iter()
            .filter(|r| !r.value().is_deprecated(now))
            .map(|r| r.value().clone())
            .collect())
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sample_identity() -> ModelIdentity {
        ModelIdentity::hosted("openai", "gpt-4-turbo", "2024-04-09")
    }

    fn sample_model() -> RegisteredModel {
        RegisteredModel::hosted(sample_identity(), 1000.0, 3000.0, 1)
    }

    #[tokio::test]
    async fn fresh_registry_empty() {
        let r = InMemoryModelRegistry::new();
        assert!(r.is_empty());
    }

    #[tokio::test]
    async fn lookup_unknown_returns_not_registered() {
        let r = InMemoryModelRegistry::new();
        let err = r.lookup(&sample_identity()).await.unwrap_err();
        assert!(matches!(err, ModelRegistryError::NotRegistered { .. }));
    }

    #[tokio::test]
    async fn register_then_lookup_succeeds() {
        let r = InMemoryModelRegistry::new();
        r.register(sample_model()).await.unwrap();
        let got = r.lookup(&sample_identity()).await.unwrap();
        assert_eq!(got.identity, sample_identity());
    }

    #[tokio::test]
    async fn register_overwrites_existing() {
        let r = InMemoryModelRegistry::new();
        r.register(sample_model()).await.unwrap();
        let mut m2 = sample_model();
        m2.price_per_million_input_tokens_cents = 9999.0;
        r.register(m2).await.unwrap();
        let got = r.lookup(&sample_identity()).await.unwrap();
        assert_eq!(got.price_per_million_input_tokens_cents, 9999.0);
    }

    #[tokio::test]
    async fn deprecate_marks_model_unavailable() {
        let r = InMemoryModelRegistry::new();
        r.register(sample_model()).await.unwrap();
        r.deprecate(&sample_identity(), SystemTime::now() - Duration::from_secs(1))
            .await
            .unwrap();
        let err = r.lookup(&sample_identity()).await.unwrap_err();
        assert!(matches!(err, ModelRegistryError::Deprecated { .. }));
    }

    #[tokio::test]
    async fn deprecate_future_does_not_block() {
        let r = InMemoryModelRegistry::new();
        r.register(sample_model()).await.unwrap();
        r.deprecate(&sample_identity(), SystemTime::now() + Duration::from_secs(3600))
            .await
            .unwrap();
        // Still lookup-able for now (deprecation date in future).
        let got = r.lookup(&sample_identity()).await;
        assert!(got.is_ok());
    }

    #[tokio::test]
    async fn list_active_excludes_deprecated() {
        let r = InMemoryModelRegistry::new();
        let m1 = sample_model();
        let mut m2_id = sample_identity();
        m2_id.requested_version = "older".into();
        let m2 = RegisteredModel::hosted(m2_id.clone(), 500.0, 1500.0, 1);
        r.register(m1).await.unwrap();
        r.register(m2).await.unwrap();
        r.deprecate(&m2_id, SystemTime::now() - Duration::from_secs(1))
            .await
            .unwrap();
        let active = r.list_active().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].identity, sample_identity());
    }

    #[tokio::test]
    async fn verify_hash_ok_when_no_expected_hash() {
        let r = InMemoryModelRegistry::new();
        r.register(sample_model()).await.unwrap();
        let actual = Blake3Hash::from_bytes([1u8; 32]);
        assert!(r.verify_hash(&sample_identity(), &actual).await.is_ok());
    }

    #[tokio::test]
    async fn verify_hash_mismatch_for_local_model() {
        let r = InMemoryModelRegistry::new();
        let mut m = sample_model();
        m.expected_content_hash = Some(Blake3Hash::from_bytes([0xAA; 32]));
        r.register(m).await.unwrap();
        let actual = Blake3Hash::from_bytes([0xBB; 32]);
        let err = r.verify_hash(&sample_identity(), &actual).await.unwrap_err();
        assert!(matches!(err, ModelRegistryError::HashMismatch { .. }));
    }

    #[tokio::test]
    async fn verify_hash_matches_expected() {
        let r = InMemoryModelRegistry::new();
        let mut m = sample_model();
        let h = Blake3Hash::from_bytes([0xCD; 32]);
        m.expected_content_hash = Some(h);
        r.register(m).await.unwrap();
        assert!(r.verify_hash(&sample_identity(), &h).await.is_ok());
    }

    #[tokio::test]
    async fn dev_skip_hash_check_bypasses_verification() {
        let r = InMemoryModelRegistry::new();
        let mut m = sample_model();
        m.expected_content_hash = Some(Blake3Hash::from_bytes([0xAA; 32]));
        m.dev_skip_hash_check = true;
        r.register(m).await.unwrap();
        let actual = Blake3Hash::from_bytes([0xBB; 32]);
        // Despite hash mismatch, skip flag wins.
        assert!(r.verify_hash(&sample_identity(), &actual).await.is_ok());
    }

    #[tokio::test]
    async fn verify_hash_on_unknown_returns_not_registered() {
        let r = InMemoryModelRegistry::new();
        let err = r
            .verify_hash(&sample_identity(), &Blake3Hash::ZERO)
            .await
            .unwrap_err();
        assert!(matches!(err, ModelRegistryError::NotRegistered { .. }));
    }

    #[test]
    fn compute_cost_cents_basic() {
        let m = sample_model();
        // 1M input, 1M output → 1000 + 3000 = 4000 cents.
        let (cost, capped) = m.compute_cost_cents(1_000_000, 1_000_000);
        assert!((cost - 4000.0).abs() < 1e-6);
        assert!(!capped);
    }

    #[test]
    fn compute_cost_cents_zero_tokens() {
        let m = sample_model();
        let (cost, capped) = m.compute_cost_cents(0, 0);
        assert_eq!(cost, 0.0);
        assert!(!capped);
    }

    #[test]
    fn compute_cost_cents_pathological_overflow_caps() {
        let mut m = sample_model();
        m.price_per_million_input_tokens_cents = f64::MAX / 2.0;
        let (cost, capped) = m.compute_cost_cents(u32::MAX, u32::MAX);
        assert!(capped);
        assert!(cost <= f64::MAX / 2.0);
    }

    #[tokio::test]
    async fn deprecate_unknown_returns_not_registered() {
        let r = InMemoryModelRegistry::new();
        let err = r
            .deprecate(&sample_identity(), SystemTime::now())
            .await
            .unwrap_err();
        assert!(matches!(err, ModelRegistryError::NotRegistered { .. }));
    }

    #[test]
    fn registered_model_serdes() {
        let m = sample_model();
        let json = serde_json::to_string(&m).unwrap();
        let m2: RegisteredModel = serde_json::from_str(&json).unwrap();
        assert_eq!(m, m2);
    }

    #[test]
    fn debug_format_does_not_panic() {
        let r = InMemoryModelRegistry::new();
        let _ = format!("{r:?}");
    }
}
