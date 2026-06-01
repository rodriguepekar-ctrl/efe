//! `efe-active-learning` — M24 closed-loop active learning.
//!
//! Pipeline :
//!   1. OBSERVE  — production classify (real-time, free)
//!   2. SELECT   — uncertainty sampler (`sampler.rs`)
//!   3. LABEL    — LLM oracle (`llm_worker.rs`) + human review (`review.rs`)
//!   4. RETUNE   — centroid recompute (`retune.rs` + `centroid.rs`)
//!   5. PROMOTE  — eval + canary + auto-promote gate (`eval.rs` + `promotion.rs`)
//!
//! All functionality is gated behind `EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED`
//! and requires M21 + M23 to be ON. See § 30 of the architecture doc for
//! the full activation flag matrix.

pub mod audit;
pub mod auto_confirm;
pub mod centroid;
pub mod cost;
pub mod error;
pub mod eval;
pub mod gdpr;
pub mod holdout;
pub mod llm_worker;
pub mod metrics;
pub mod promotion;
pub mod queue;
pub mod retune;
pub mod review;
pub mod sampler;
pub mod service;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_helpers;

pub use error::{ActiveLearningError, ActiveLearningResult};

/// Crate version. Bump on architectural change ; not the gRPC service
/// version (which is `v1` and stable).
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

// Re-export commonly used types from efe-curation (M23) so callers
// don't need a second import.
pub use efe_curation::{CatalogId, ClassId, TenantId};

/// Feature flag check. Reads server-wide flag + tenant flag.
pub fn is_enabled_for_tenant(tenant_features: &TenantActiveLearningFlags) -> bool {
    let server_on = std::env::var("EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    server_on && tenant_features.active_learning_enabled
}

/// Subset of tenant feature flags relevant to M24. Resolved per-call.
#[derive(Debug, Clone)]
pub struct TenantActiveLearningFlags {
    pub active_learning_enabled: bool,
    pub auto_promotion_enabled: bool,
    pub human_review_required: bool,
    pub dpo_consent_active: bool,
    pub uncertainty_threshold: f64,
    pub auto_confirm_threshold: f64,
    pub queue_max_size: u32,
    pub gold_label_threshold: u32,
    pub retune_interval_days: u32,
    pub anchor_weight_alpha: f64,
    pub max_centroid_drift: f64,
    pub promotion_threshold_pp: f64,
    pub active_learning_budget_cents_per_day: i64,
    pub review_sla_hours: u32,
    pub notify_after_hours: u32,
}

impl Default for TenantActiveLearningFlags {
    fn default() -> Self {
        Self {
            active_learning_enabled: false,
            auto_promotion_enabled: false,
            human_review_required: true,
            dpo_consent_active: false,
            uncertainty_threshold: 0.70,
            auto_confirm_threshold: 0.90,
            queue_max_size: 10_000,
            gold_label_threshold: 1_000,
            retune_interval_days: 7,
            anchor_weight_alpha: 0.70,
            max_centroid_drift: 0.30,
            promotion_threshold_pp: 1.00,
            active_learning_budget_cents_per_day: 0,
            review_sla_hours: 48,
            notify_after_hours: 8,
        }
    }
}

/// Stakes tier of a catalog — controls auto-promotion eligibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::Type)]
#[sqlx(type_name = "stakes_tier", rename_all = "UPPERCASE")]
pub enum StakesTier { Low, Medium, High }

impl StakesTier {
    pub fn as_str(&self) -> &'static str {
        match self { Self::Low => "LOW", Self::Medium => "MEDIUM", Self::High => "HIGH" }
    }
}

/// Validate startup dependencies — M24 requires M21 and M23 ON.
pub fn validate_dependencies() -> Result<(), String> {
    let m24 = env_bool("EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED");
    if !m24 { return Ok(()); }
    let require = env_bool_default("EFE_FEATURE_M24_REQUIRE_M21_M23", true);
    if !require {
        tracing::warn!("M24 ON without M21/M23 dep validation — operator override");
        return Ok(());
    }
    let m21 = env_bool("EFE_FEATURE_M21_LLMROUTER");
    let m23 = env_bool("EFE_FEATURE_M23_CURATION_ENABLED");
    if !m21 || !m23 {
        return Err(format!(
            "M24 requires M21={m21} and M23={m23} (both must be true) ; \
             set EFE_FEATURE_M24_REQUIRE_M21_M23=false to override (not recommended)"
        ));
    }
    Ok(())
}

fn env_bool(key: &str) -> bool {
    matches!(std::env::var(key).as_deref(), Ok("true") | Ok("1") | Ok("yes"))
}

fn env_bool_default(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => matches!(v.as_str(), "true" | "1" | "yes"),
        Err(_) => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Φ-D-OPS post-audit round-2 fix B4' : the test below mutates
    /// `EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED` (process-global).
    /// Cargo runs unit tests in parallel — any sibling test reading
    /// the same var (or anything else `env_bool`-derived) sees a
    /// transient absent state. Mirrors the `ENV_LOCK` pattern from
    /// `efe-pii/src/presidio_client.rs` and `efe-server::worker.rs`.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn validate_off_returns_ok() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Save / restore env var to avoid test cross-contamination.
        std::env::remove_var("EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED");
        assert!(validate_dependencies().is_ok());
    }

    #[test]
    fn flags_default_safe() {
        let f = TenantActiveLearningFlags::default();
        assert!(!f.active_learning_enabled);
        assert!(!f.auto_promotion_enabled);
        assert!(f.human_review_required);
        assert!(!f.dpo_consent_active);
    }

    #[test]
    fn stakes_tier_serialization() {
        assert_eq!(StakesTier::Low.as_str(), "LOW");
        assert_eq!(StakesTier::Medium.as_str(), "MEDIUM");
        assert_eq!(StakesTier::High.as_str(), "HIGH");
    }
}
