// SPDX-License-Identifier: UNLICENSED
//! **Sprint 16 — N24 + I20 + Annex A2** :
//! `WiredComplianceRunner` — single public factory that wires
//! the three gates + cache + grace + metrics into a
//! `NanoToSwarmPolicy`.
//!
//! Per I20 there is exactly ONE public constructor :
//! [`WiredComplianceRunner::new`].  Every other knob lives in
//! [`WiredComplianceConfig`] (also single-public-shape).

use std::sync::Arc;
use std::time::Duration;

use efe_gold_test_gate::GoldApi;
use efe_nano_sdk::{NanoAgent, PublisherId, PublisherIdentity, StabilityTier};
use efe_nano_swarm_adapter::{
    GoldBindingGate, IdempotencyCache, InMemoryIdempotencyCache, NanoToSwarmPolicy,
    PublisherStatusGate, SignatureGate,
};
use efe_publisher_identity::registry::PublisherRegistry;
use efe_publisher_identity::verifier::SignatureVerifier;
use serde::{Deserialize, Serialize};

use crate::builtin::EFE_BUILTIN_PUBLISHER_IDENTITY;
use crate::cache::{PublisherIdentityCache, DEFAULT_CACHE_CAPACITY, DEFAULT_CACHE_TTL_SECS};
use crate::gates::{WiredGoldBindingGate, WiredPublisherStatusGate, WiredSignatureGate};
use crate::grace::{DenyAllGrace, GraceWindowResolver};
use crate::metrics::{N24ComplianceMetrics, N24MetricsSnapshot};

/// **N24** : default registry-lookup timeout (500 ms).
pub const DEFAULT_LOOKUP_TIMEOUT_MS: u32 = 500;

/// **N24** : runner configuration ; cheap to clone.
#[derive(Clone)]
pub struct WiredComplianceConfig {
    /// LRU + TTL cache capacity (default 1024).
    pub cache_capacity: usize,
    /// LRU + TTL cache entry TTL (default 60 s).
    pub cache_ttl: Duration,
    /// Per-lookup timeout against the publisher registry
    /// (default 500 ms).
    pub lookup_timeout: Duration,
    /// Optional override of the built-in EFE publisher identity.
    /// `None` → use the baked-in
    /// [`crate::builtin::EFE_BUILTIN_PUBLISHER_IDENTITY`].
    pub builtin_publisher_override: Option<PublisherIdentity>,
}

impl Default for WiredComplianceConfig {
    fn default() -> Self {
        Self {
            cache_capacity: DEFAULT_CACHE_CAPACITY,
            cache_ttl: Duration::from_secs(DEFAULT_CACHE_TTL_SECS),
            lookup_timeout: Duration::from_millis(DEFAULT_LOOKUP_TIMEOUT_MS as u64),
            builtin_publisher_override: None,
        }
    }
}

impl std::fmt::Debug for WiredComplianceConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WiredComplianceConfig")
            .field("cache_capacity", &self.cache_capacity)
            .field("cache_ttl", &self.cache_ttl)
            .field("lookup_timeout", &self.lookup_timeout)
            .field(
                "builtin_publisher_override",
                &self.builtin_publisher_override.is_some(),
            )
            .finish()
    }
}

/// **N24 + I20** : the wired compliance runner — single public
/// factory + facade.
pub struct WiredComplianceRunner {
    config: WiredComplianceConfig,
    publisher_registry: Arc<dyn PublisherRegistry>,
    verifier: Arc<dyn SignatureVerifier>,
    gold_api: Arc<dyn GoldApi>,
    cache: Arc<PublisherIdentityCache>,
    grace: Arc<dyn GraceWindowResolver>,
    metrics: Arc<N24ComplianceMetrics>,
    builtin_identity: PublisherIdentity,

    // Pre-built gate Arcs so every policy clone shares one
    // instance (no per-request allocation).
    signature_gate: Arc<dyn SignatureGate>,
    gold_gate: Arc<dyn GoldBindingGate>,
    status_gate: Arc<dyn PublisherStatusGate>,
    idempotency_cache: Arc<dyn IdempotencyCache>,
}

impl std::fmt::Debug for WiredComplianceRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WiredComplianceRunner")
            .field("config", &self.config)
            .field("cache_len", &self.cache.len())
            .field(
                "builtin_publisher_id",
                &self.builtin_identity.publisher_id,
            )
            .finish_non_exhaustive()
    }
}

impl WiredComplianceRunner {
    /// **N24 + I20** : the only public factory.
    pub fn new(
        config: WiredComplianceConfig,
        publisher_registry: Arc<dyn PublisherRegistry>,
        verifier: Arc<dyn SignatureVerifier>,
        gold_api: Arc<dyn GoldApi>,
        grace: Arc<dyn GraceWindowResolver>,
        metrics: Arc<N24ComplianceMetrics>,
    ) -> Self {
        let builtin_identity = config
            .builtin_publisher_override
            .clone()
            .unwrap_or_else(EFE_BUILTIN_PUBLISHER_IDENTITY);
        let cache = Arc::new(PublisherIdentityCache::new(
            config.cache_capacity,
            config.cache_ttl,
        ));
        let signature_gate: Arc<dyn SignatureGate> = Arc::new(WiredSignatureGate::new(
            publisher_registry.clone(),
            verifier.clone(),
            cache.clone(),
            grace.clone(),
            metrics.clone(),
            config.lookup_timeout,
            builtin_identity.clone(),
            None,
        ));
        let gold_gate: Arc<dyn GoldBindingGate> = Arc::new(WiredGoldBindingGate::new(
            gold_api.clone(),
            metrics.clone(),
        ));
        let status_gate: Arc<dyn PublisherStatusGate> = Arc::new(WiredPublisherStatusGate::new(
            publisher_registry.clone(),
            cache.clone(),
            metrics.clone(),
            config.lookup_timeout,
            builtin_identity.clone(),
        ));
        let idempotency_cache: Arc<dyn IdempotencyCache> =
            Arc::new(InMemoryIdempotencyCache::new());

        Self {
            config,
            publisher_registry,
            verifier,
            gold_api,
            cache,
            grace,
            metrics,
            builtin_identity,
            signature_gate,
            gold_gate,
            status_gate,
            idempotency_cache,
        }
    }

    /// **N24** : convenience constructor that wires the default
    /// config + a baked grace store (DenyAll).
    pub fn with_defaults(
        publisher_registry: Arc<dyn PublisherRegistry>,
        verifier: Arc<dyn SignatureVerifier>,
        gold_api: Arc<dyn GoldApi>,
    ) -> Self {
        Self::new(
            WiredComplianceConfig::default(),
            publisher_registry,
            verifier,
            gold_api,
            Arc::new(DenyAllGrace),
            Arc::new(N24ComplianceMetrics::new()),
        )
    }

    // ─── Policy factories ─────────────────────────────────────────

    /// **N24 + Annex A2** : full Tier-1+ wired policy.  All three
    /// gates ON.  Use for `Tier1`, `Tier2`, `TierAudit` catalogue
    /// entries.
    pub fn tier1_strict_policy(&self) -> NanoToSwarmPolicy {
        NanoToSwarmPolicy::tier1_strict_wired(
            self.signature_gate.clone(),
            self.gold_gate.clone(),
            self.status_gate.clone(),
            self.idempotency_cache.clone(),
        )
    }

    /// **N24 + Annex A2** : relaxed policy for TierExperimental
    /// entries — status gate ON, signature + gold OFF.
    pub fn experimental_relaxed_policy(&self) -> NanoToSwarmPolicy {
        NanoToSwarmPolicy {
            verify_signature: false,
            require_gold_binding: false,
            require_publisher_status: true,
            enforce_limits: true,
            idempotency_cache: Some(self.idempotency_cache.clone()),
            signature_gate: None,
            gold_gate: None,
            publisher_status_gate: Some(self.status_gate.clone()),
            idempotency_ttl_override: None,
        }
    }

    /// **N24 + Annex A3** : pick the right policy for a catalogue
    /// entry based on its stability tier.
    ///
    /// Per-entry override : if the nano declares NO
    /// `gold_set_ref`, the `require_gold_binding` flag is forced
    /// to `false` regardless of tier.  This unblocks the Sprint
    /// 16 default DAG (built-in catalogue entries don't ship
    /// approved gold sets yet ; Sprint 17 lights up the gold
    /// admin endpoint that approves them).  Once the entry's
    /// `gold_set_ref` is populated, the gate fires automatically.
    pub fn select_policy_for(&self, nano: &NanoAgent) -> NanoToSwarmPolicy {
        let mut policy = match nano.stability_tier {
            StabilityTier::Tier1 | StabilityTier::Tier2 | StabilityTier::TierAudit => {
                self.tier1_strict_policy()
            }
            StabilityTier::TierExperimental => self.experimental_relaxed_policy(),
        };
        if nano.gold_set_ref.is_none() {
            policy.require_gold_binding = false;
        }
        policy
    }

    // ─── Cache management ─────────────────────────────────────────

    /// **N24 + G12 + I14** : explicit cache eviction (called by
    /// the gateway's `revoke_publisher_handler` to invalidate the
    /// cache on revoke / suspend).
    pub fn evict_publisher(&self, publisher_id: PublisherId) -> bool {
        self.cache.evict(publisher_id)
    }

    /// **N24** : evict every cache entry.
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    // ─── Accessors ────────────────────────────────────────────────

    /// **N24** : current config (snapshot).
    pub fn config(&self) -> &WiredComplianceConfig {
        &self.config
    }

    /// **N24** : the baked-in publisher identity used to verify
    /// the built-in catalogue.
    pub fn builtin_publisher_identity(&self) -> &PublisherIdentity {
        &self.builtin_identity
    }

    /// **N24** : direct access to the metrics struct.
    pub fn metrics(&self) -> &N24ComplianceMetrics {
        &self.metrics
    }

    /// **N24** : direct access to the cache (mostly for tests).
    pub fn cache(&self) -> &PublisherIdentityCache {
        &self.cache
    }

    /// **N24** : direct access to the underlying publisher
    /// registry (used by the runner's owner — usually the
    /// gateway's `revoke_publisher_handler` — to fan out an
    /// invalidation).
    pub fn publisher_registry(&self) -> &Arc<dyn PublisherRegistry> {
        &self.publisher_registry
    }

    /// **N24** : direct access to the underlying verifier.
    pub fn verifier(&self) -> &Arc<dyn SignatureVerifier> {
        &self.verifier
    }

    /// **N24** : direct access to the underlying gold api.
    pub fn gold_api(&self) -> &Arc<dyn GoldApi> {
        &self.gold_api
    }

    /// **N24** : direct access to the underlying grace resolver.
    pub fn grace(&self) -> &Arc<dyn GraceWindowResolver> {
        &self.grace
    }

    // ─── Compliance status snapshot (admin endpoint) ──────────────

    /// **N24 + G13** : live snapshot for the admin endpoint.
    pub fn compliance_status(&self) -> ComplianceStatus {
        let snap = self.metrics.snapshot();
        ComplianceStatus {
            schema: SCHEMA_LITERAL.to_string(),
            generated_at: chrono::Utc::now().to_rfc3339(),
            cache: ComplianceStatusCache {
                len: self.cache.len() as u64,
                capacity: self.cache.capacity() as u64,
                hits: self.cache.hit_count(),
                misses: self.cache.miss_count(),
                evicted: self.cache.evicted_count(),
                expired: self.cache.expired_count(),
                hit_ratio: self.cache.hit_ratio(),
            },
            counters: ComplianceStatusCounters {
                signature_verified: snap.signature.ok,
                signature_failed: snap.signature.verify_failed
                    + snap.signature.author_mismatch
                    + snap.signature.publisher_not_found
                    + snap.signature.key_not_found
                    + snap.signature.missing
                    + snap.signature.other,
                signature_grace_skipped: snap.signature.grace_skip,
                gold_bound: snap.gold.ok,
                gold_failed: snap.gold.not_approved
                    + snap.gold.hash_drift
                    + snap.gold.lookup_failed
                    + snap.gold.no_ref
                    + snap.gold.other,
                gold_grace_skipped: snap.gold.grace_skip,
                status_active: snap.status.ok,
                status_blocked: snap.status.suspended
                    + snap.status.revoked
                    + snap.status.sanctions_check_stale,
                status_not_publisher: snap.status.not_publisher,
                status_publisher_not_found: snap.status.publisher_not_found,
                status_registry_timeout: snap.status.registry_timeout,
            },
            gates: ComplianceStatusGates {
                signature: snap.signature,
                gold: snap.gold,
                status: snap.status,
            },
            builtin: ComplianceStatusBuiltin {
                publisher_id: self.builtin_identity.publisher_id,
                key_id: self.builtin_identity.active_key.key_id,
                kyc_tier: self.builtin_identity.kyc_tier.label().to_string(),
            },
        }
    }
}

/// **N24 + G13** : wire schema literal for the admin endpoint.
pub const SCHEMA_LITERAL: &str = "compliance.status.v1";

/// **N24 + G13** : payload returned by `GET /v1/admin/compliance-status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub struct ComplianceStatus {
    /// Wire schema literal — always `"compliance.status.v1"`.
    pub schema: String,
    /// UTC RFC-3339 emission timestamp.
    pub generated_at: String,
    /// Cache state.
    pub cache: ComplianceStatusCache,
    /// Rolled-up gate counters.
    pub counters: ComplianceStatusCounters,
    /// Per-gate detailed counters (annex A5).
    pub gates: ComplianceStatusGates,
    /// Built-in publisher identity snapshot.
    pub builtin: ComplianceStatusBuiltin,
}

/// **N24** : cache state subsection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComplianceStatusCache {
    /// Current number of cached publishers.
    pub len: u64,
    /// LRU cap.
    pub capacity: u64,
    /// Cumulative hit count.
    pub hits: u64,
    /// Cumulative miss count.
    pub misses: u64,
    /// Cumulative explicit + capacity-driven evictions.
    pub evicted: u64,
    /// Cumulative TTL-expired evictions.
    pub expired: u64,
    /// Live ratio in `[0.0, 1.0]`.
    pub hit_ratio: f64,
}

/// **N24** : rolled-up counters (admin-friendly).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComplianceStatusCounters {
    /// Signature gate OK count.
    pub signature_verified: u64,
    /// Signature gate failed count (every error variant summed).
    pub signature_failed: u64,
    /// Signature gate grace-skipped count.
    pub signature_grace_skipped: u64,
    /// Gold gate OK count.
    pub gold_bound: u64,
    /// Gold gate failed count.
    pub gold_failed: u64,
    /// Gold gate grace-skipped count.
    pub gold_grace_skipped: u64,
    /// Status gate active count.
    pub status_active: u64,
    /// Status gate blocked count (suspended + revoked + sanctions stale).
    pub status_blocked: u64,
    /// Status gate "not a publisher" skip count.
    pub status_not_publisher: u64,
    /// Status gate publisher-not-found count.
    pub status_publisher_not_found: u64,
    /// Status gate registry-timeout count.
    pub status_registry_timeout: u64,
}

/// **N24 + Annex A5** : per-gate counters (full breakdown).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComplianceStatusGates {
    /// Full signature gate breakdown.
    pub signature: crate::metrics::SignatureSnapshot,
    /// Full gold gate breakdown.
    pub gold: crate::metrics::GoldSnapshot,
    /// Full status gate breakdown.
    pub status: crate::metrics::StatusSnapshot,
}

/// **N24** : built-in publisher identity snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComplianceStatusBuiltin {
    /// Baked-in publisher id.
    pub publisher_id: PublisherId,
    /// Baked-in active key id.
    pub key_id: efe_nano_sdk::KeyId,
    /// Baked-in publisher KYC tier (always Tier3).
    pub kyc_tier: String,
}

impl ComplianceStatus {
    /// **N24 + G13** : the wire schema literal.
    pub fn schema() -> &'static str {
        SCHEMA_LITERAL
    }
}

// Helper: ensure N24MetricsSnapshot is reachable through this
// module (used by serde tests in tests/).  Public re-export above
// keeps the symbol stable.
#[allow(dead_code)]
fn _ensure_metrics_snapshot_reachable() -> Option<N24MetricsSnapshot> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_publisher_identity::registry::InMemoryPublisherRegistry;
    use efe_publisher_identity::verifier::Ed25519Verifier;

    // A null GoldApi used for compile-time wiring tests ; never
    // dispatched.
    struct NullGoldApi;
    #[async_trait::async_trait]
    impl GoldApi for NullGoldApi {
        async fn register_gold_set(
            &self,
            _req: efe_gold_test_gate::api::RegisterGoldSetRequest,
        ) -> Result<efe_nano_sdk::GoldSetId, efe_gold_test_gate::GoldApiError> {
            unimplemented!()
        }
        async fn approve_gold_set(
            &self,
            _: efe_nano_sdk::GoldSetId,
            _: efe_gold_test_gate::api::ApproveGoldSetRequest,
        ) -> Result<efe_nano_sdk::GoldSet, efe_gold_test_gate::GoldApiError> {
            unimplemented!()
        }
        async fn lookup_gold_set(
            &self,
            gold_set_id: efe_nano_sdk::GoldSetId,
        ) -> Result<efe_nano_sdk::GoldSet, efe_gold_test_gate::GoldApiError> {
            { let _ = gold_set_id; Err(efe_gold_test_gate::GoldApiError::NotFound) }
        }
        async fn list_gold_sets_for_agent(
            &self,
            _: &efe_nano_sdk::NanoAgentId,
        ) -> Result<Vec<efe_nano_sdk::GoldSet>, efe_gold_test_gate::GoldApiError> {
            Ok(vec![])
        }
        async fn run_gate(
            &self,
            _: efe_gold_test_gate::api::RunRequest,
        ) -> Result<efe_nano_sdk::GoldReport, efe_gold_test_gate::GoldApiError> {
            unimplemented!()
        }
        async fn lookup_gold_run(
            &self,
            _: efe_nano_sdk::GoldRunId,
        ) -> Result<Option<efe_gold_test_gate::GoldRunLedgerRow>, efe_gold_test_gate::GoldApiError> {
            unimplemented!()
        }
        async fn drift_sweep(
            &self,
            _: efe_gold_test_gate::api::DriftSweepRequest,
        ) -> Result<efe_gold_test_gate::api::DriftSweepReport, efe_gold_test_gate::GoldApiError> {
            unimplemented!()
        }
    }

    fn runner() -> WiredComplianceRunner {
        WiredComplianceRunner::with_defaults(
            Arc::new(InMemoryPublisherRegistry::new()),
            Arc::new(Ed25519Verifier::new()),
            Arc::new(NullGoldApi),
        )
    }

    #[test]
    fn default_config_uses_documented_constants() {
        let c = WiredComplianceConfig::default();
        assert_eq!(c.cache_capacity, DEFAULT_CACHE_CAPACITY);
        assert_eq!(c.cache_ttl, Duration::from_secs(DEFAULT_CACHE_TTL_SECS));
        assert_eq!(
            c.lookup_timeout,
            Duration::from_millis(DEFAULT_LOOKUP_TIMEOUT_MS as u64)
        );
        assert!(c.builtin_publisher_override.is_none());
    }

    #[test]
    fn tier1_strict_policy_all_gates_on() {
        let r = runner();
        let p = r.tier1_strict_policy();
        assert!(p.verify_signature);
        assert!(p.require_gold_binding);
        assert!(p.require_publisher_status);
        assert!(p.signature_gate.is_some());
        assert!(p.gold_gate.is_some());
        assert!(p.publisher_status_gate.is_some());
        assert!(p.idempotency_cache.is_some());
    }

    #[test]
    fn experimental_relaxed_policy_status_only() {
        let r = runner();
        let p = r.experimental_relaxed_policy();
        assert!(!p.verify_signature);
        assert!(!p.require_gold_binding);
        assert!(p.require_publisher_status);
        assert!(p.signature_gate.is_none());
        assert!(p.gold_gate.is_none());
        assert!(p.publisher_status_gate.is_some());
        assert!(p.idempotency_cache.is_some());
    }

    #[test]
    fn select_policy_for_each_tier() {
        use efe_nano_preanalysis::default_catalogue;
        let r = runner();
        for (nano, _) in default_catalogue() {
            let p = r.select_policy_for(&nano);
            match nano.stability_tier {
                StabilityTier::TierExperimental => {
                    assert!(!p.verify_signature, "{}", nano.id.as_str());
                    assert!(p.require_publisher_status);
                }
                _ => {
                    assert!(p.verify_signature, "{}", nano.id.as_str());
                    assert!(p.require_publisher_status);
                    // Per-entry override : entries with no
                    // `gold_set_ref` don't fire the gold gate
                    // (Sprint 16 catalogue ships without baked
                    // gold sets ; Sprint 17 lights them up).
                    assert_eq!(p.require_gold_binding, nano.gold_set_ref.is_some());
                }
            }
        }
    }

    #[test]
    fn select_policy_for_entry_with_gold_set_ref_enables_gold_gate() {
        use chrono::Utc;
        use efe_nano_sdk::{
            Author, Category, DeterminismLevel, ExecutionMode, GoldSetId, GoldSetRef,
            IdempotencyPolicy, NanoAgentId, NanoAgentVersion, ResourceLimits, Sha256Hash,
        };
        let r = runner();
        let nano = NanoAgent {
            id: NanoAgentId::new("with_gold").unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "x".into(),
            author: Author::Efe,
            category: Category::Validation,
            stability_tier: StabilityTier::Tier1,
            execution_mode: ExecutionMode::PureRust {
                entry: "x".into(),
            },
            determinism: DeterminismLevel::FullDeterministic,
            idempotency: IdempotencyPolicy::auto_24h(),
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            error_schema: None,
            limits: ResourceLimits::default_mode_a(),
            cost_estimate_cents: 0.01,
            latency_p99_ms_target: 50,
            audit: Default::default(),
            compliance: Default::default(),
            mcp_tool_metadata: None,
            signature: None,
            co_publishing: None,
            gold_set_ref: Some(GoldSetRef {
                gold_set_id: GoldSetId::new(),
                gold_set_hash: Sha256Hash::from_bytes([0u8; 32]),
            }),
            created_at: Utc::now(),
        };
        let p = r.select_policy_for(&nano);
        // gold_set_ref present → gate fires.
        assert!(p.require_gold_binding);
    }

    #[test]
    fn evict_publisher_returns_false_for_unknown() {
        let r = runner();
        let id = PublisherId::new();
        assert!(!r.evict_publisher(id));
    }

    #[test]
    fn evict_publisher_returns_true_when_present() {
        let r = runner();
        let id = PublisherId::new();
        // Seed cache directly.
        r.cache().put(
            id,
            crate::builtin::EFE_BUILTIN_PUBLISHER_IDENTITY(),
        );
        assert!(r.evict_publisher(id));
    }

    #[test]
    fn compliance_status_schema_literal_stable() {
        let r = runner();
        let s = r.compliance_status();
        assert_eq!(s.schema, "compliance.status.v1");
        assert!(s.generated_at.contains('T'));
    }

    #[test]
    fn compliance_status_serde_round_trip() {
        let r = runner();
        let s = r.compliance_status();
        let json = serde_json::to_string(&s).unwrap();
        let back: ComplianceStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn builtin_identity_accessor_matches_construction() {
        let r = runner();
        let id = r.builtin_publisher_identity();
        assert_eq!(id.publisher_id, EFE_BUILTIN_PUBLISHER_IDENTITY().publisher_id);
    }

    #[test]
    fn clear_cache_empties_it() {
        let r = runner();
        let id = PublisherId::new();
        r.cache().put(
            id,
            crate::builtin::EFE_BUILTIN_PUBLISHER_IDENTITY(),
        );
        assert_eq!(r.cache().len(), 1);
        r.clear_cache();
        assert_eq!(r.cache().len(), 0);
    }
}
