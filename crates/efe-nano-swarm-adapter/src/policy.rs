// SPDX-License-Identifier: UNLICENSED
//! **Sprint 15 — N23 + ADR-3** : compliance + caching policy
//! applied around every adapter dispatch.
//!
//! The policy is **per-construction** : a process may host many
//! `NanoToSwarm` adapters configured differently (e.g. Tier-1 EFE
//! built-ins under strict policy, tenant experimentals under fast
//! policy).  This matches ADR-3 — policy is an argument, not a
//! global.
//!
//! ## Gate composition
//!
//! 1. **`SignatureGate`** (G2) — verifies the catalogue entry's
//!    `PublisherSignature` against the publisher registry.  When
//!    `verify_signature == true` and the gate is `None`, dispatch
//!    fails closed (F1).
//! 2. **`GoldBindingGate`** (G4) — looks up the catalogue's
//!    `gold_set_ref` and asserts the gold set is `Approved`.
//!    When `require_gold_binding == true` and the gate is `None`,
//!    dispatch fails closed (F2).
//! 3. **`IdempotencyCache`** (G5) — hardened-key cache (`agent_id`,
//!    `version`, canonical input hash).  Hit returns the cached
//!    `NanoAgentResponse` without invoking the runner.  Misses
//!    insert with TTL.
//!
//! All gates are optional `Arc<dyn ...>` ; callers wire only what
//! they need.  Two convenience constructors :
//!
//! - [`NanoToSwarmPolicy::experimental_fast`] — all gates off,
//!   adapter delivers raw dispatch + envelope only.
//! - [`NanoToSwarmPolicy::tier1_strict`] — all gates wired, hard
//!   failure if any precondition is missing.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use dashmap::DashMap;
use efe_nano_sdk::{NanoAgent, PublisherSignature};
use efe_nano_sdk::traits::runtime::NanoAgentResponse;
use sha2::{Digest, Sha256};

use crate::error::AdapterError;

// ─────────────────────────────────────────────────────────────────────
//  Signature gate
// ─────────────────────────────────────────────────────────────────────

/// **N23 + G2** : signature verification gate.
///
/// A thin trait the adapter calls when `policy.verify_signature
/// == true`.  Implementations typically wrap
/// `efe_publisher_identity::SignatureVerifier` ; we keep this
/// shape so the adapter does NOT pull `efe-publisher-identity` as
/// a hard dependency.
#[async_trait]
pub trait SignatureGate: Send + Sync + 'static {
    /// **G2** : verify the catalogue `nano`'s `signature` against
    /// the gate's identity registry.  Returns `Ok(())` on success,
    /// `Err(reason)` on any failure (forgery, key not found,
    /// algorithm mismatch).
    async fn verify(
        &self,
        nano: &NanoAgent,
        signature: &PublisherSignature,
    ) -> Result<(), String>;
}

// ─────────────────────────────────────────────────────────────────────
//  Gold binding gate
// ─────────────────────────────────────────────────────────────────────

/// **N23 + G4** : gold binding verification gate.
///
/// Verifies the catalogue's `gold_set_ref` resolves to an
/// `Approved` gold set.  Implementations typically wrap
/// `efe_gold_test_gate::GoldApi::lookup_gold_set` and check the
/// returned status ; we keep the surface narrow so the adapter
/// stays decoupled.
#[async_trait]
pub trait GoldBindingGate: Send + Sync + 'static {
    /// **G4** : assert that `nano`'s gold binding is approved.
    /// Returns `Ok(())` on success, `Err(reason)` if the
    /// catalogue has no binding or the lookup returned a non-
    /// `Approved` status.
    async fn assert_approved(&self, nano: &NanoAgent) -> Result<(), String>;
}

// ─────────────────────────────────────────────────────────────────────
//  Publisher status gate (Sprint 16 — N24 + G4 + I12, additive)
// ─────────────────────────────────────────────────────────────────────

/// **N24 + G4** : publisher status gate.
///
/// Runs BEFORE signature + gold verification (cheap-first, I12).
/// Resolves the catalogue entry's `Author::Publisher` (if any),
/// looks up the publisher in the N18 registry, and asserts
/// `!identity.dispatch_blocked()` (i.e. not Suspended /
/// Revoked).  Catalogue entries whose author is NOT a publisher
/// (Efe / Tenant / Community) skip the check and return Ok.
///
/// This trait was added in Sprint 16 N24.  Existing N23 callers
/// don't need to implement it — the field is `Option<Arc<dyn
/// PublisherStatusGate>>` with default `None`.
#[async_trait]
pub trait PublisherStatusGate: Send + Sync + 'static {
    /// **G4** : Returns `Ok(())` if the publisher is dispatchable
    /// (or if the agent has no publisher author).  Returns
    /// `Err(reason)` if the publisher is suspended / revoked /
    /// missing / the registry lookup failed.
    async fn assert_active(&self, nano: &NanoAgent) -> Result<(), String>;
}

// ─────────────────────────────────────────────────────────────────────
//  Idempotency cache
// ─────────────────────────────────────────────────────────────────────

/// **N23 + G5** : per-key TTL'd response cache.
///
/// Implementations may be in-memory ([`InMemoryIdempotencyCache`])
/// or back a remote Redis (N12-aligned).  The adapter hashes
/// `(agent_id, version, input)` with SHA-256 to obtain the key
/// (cf. [`compute_cache_key`]).
#[async_trait]
pub trait IdempotencyCache: Send + Sync + 'static {
    /// **G5** : look up a cached response.  Returns `None` on
    /// miss.
    async fn get(&self, key: &str) -> Result<Option<NanoAgentResponse>, AdapterError>;

    /// **G5** : insert a fresh response.  TTL is enforced on
    /// reads via the wall clock.
    async fn put(
        &self,
        key: &str,
        value: NanoAgentResponse,
        ttl: Duration,
    ) -> Result<(), AdapterError>;
}

/// **N23 + G5** : compute the SHA-256 cache key (hex) for an
/// `(nano, input)` pair.
///
/// The key shape includes the catalogue id, version, and a
/// canonical JSON input hash so two different versions of the
/// same agent never collide (N12 hardening).  The catalogue
/// `cost_estimate_cents` is intentionally NOT in the key — it
/// changes between catalogue revisions but the actual response
/// is independent.
pub fn compute_cache_key(nano: &NanoAgent, input: &serde_json::Value) -> String {
    let mut h = Sha256::new();
    h.update(b"nano-cache-v1\x00");
    h.update(nano.id.as_str().as_bytes());
    h.update(b"\x00");
    h.update(nano.version.as_str().as_bytes());
    h.update(b"\x00");
    // Canonical input — `serde_json::to_vec` produces stable
    // bytes for the same Value but key ordering inside objects
    // is map-iteration order (the default `serde_json::Value`
    // uses `Map` which is `BTreeMap`-backed when the
    // `preserve_order` feature is off).  For Sprint 15 we accept
    // this — pre-analysis inputs are typically `{"text": "..."}`
    // and the BTreeMap iteration order is deterministic.
    let bytes = serde_json::to_vec(input).unwrap_or_default();
    h.update(&bytes);
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// **N23 + G5** : in-memory idempotency cache backed by a
/// `DashMap`.
///
/// Properties :
/// - Per-key TTL enforced on each `get` (lazy expiry).
/// - Concurrent-safe (DashMap shard locks).
/// - Bounded by per-process memory only ; Sprint 16+ may swap
///   for an LRU-with-cap.
pub struct InMemoryIdempotencyCache {
    map: DashMap<String, (NanoAgentResponse, SystemTime)>,
}

impl Default for InMemoryIdempotencyCache {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryIdempotencyCache {
    /// **N23** : construct an empty cache.
    pub fn new() -> Self {
        Self {
            map: DashMap::new(),
        }
    }

    /// **N23 — test helper** : how many keys currently in cache.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// **N23 — test helper** : true if empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl std::fmt::Debug for InMemoryIdempotencyCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryIdempotencyCache")
            .field("len", &self.map.len())
            .finish()
    }
}

#[async_trait]
impl IdempotencyCache for InMemoryIdempotencyCache {
    async fn get(&self, key: &str) -> Result<Option<NanoAgentResponse>, AdapterError> {
        let now = SystemTime::now();
        match self.map.get(key) {
            Some(entry) => {
                let (resp, expiry) = entry.value().clone();
                if expiry > now {
                    Ok(Some(resp))
                } else {
                    // Drop the read guard before mutating.
                    drop(entry);
                    self.map.remove(key);
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    async fn put(
        &self,
        key: &str,
        value: NanoAgentResponse,
        ttl: Duration,
    ) -> Result<(), AdapterError> {
        let expiry = SystemTime::now()
            .checked_add(ttl)
            .unwrap_or_else(|| SystemTime::now() + Duration::from_secs(60));
        self.map.insert(key.to_string(), (value, expiry));
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────
//  NanoToSwarmPolicy
// ─────────────────────────────────────────────────────────────────────

/// **N23 + ADR-3** : per-construction policy for `NanoToSwarm`.
///
/// Cheap to clone (all fields are `bool` or `Option<Arc<...>>`).
#[derive(Clone)]
pub struct NanoToSwarmPolicy {
    /// **G2** : if `true`, the adapter calls
    /// `signature_gate.verify(...)` before dispatching.
    pub verify_signature: bool,
    /// **G4** : if `true`, the adapter calls
    /// `gold_gate.assert_approved(nano)` before dispatching.
    pub require_gold_binding: bool,
    /// **N24 G4** : if `true`, the adapter calls
    /// `publisher_status_gate.assert_active(nano)` before
    /// dispatching.  Default `false` for N23-era callers.
    pub require_publisher_status: bool,
    /// **G9** : if `true`, the adapter wraps the runner dispatch
    /// in a `tokio::time::timeout(agent.limits.wall_clock_ms)`.
    pub enforce_limits: bool,
    /// **G5** : optional response cache.
    pub idempotency_cache: Option<Arc<dyn IdempotencyCache>>,
    /// **G2** : the verifier the signature gate calls.
    pub signature_gate: Option<Arc<dyn SignatureGate>>,
    /// **G4** : the gold gate the adapter consults.
    pub gold_gate: Option<Arc<dyn GoldBindingGate>>,
    /// **N24 G4** : the publisher-status gate the adapter
    /// consults (cheap-first, runs before signature + gold).
    pub publisher_status_gate: Option<Arc<dyn PublisherStatusGate>>,
    /// **G5 — TTL override** : default `None` → derived from the
    /// nano's `IdempotencyPolicy::Auto.ttl_sec`.  An explicit
    /// override applies to every cache write through this policy
    /// instance.
    pub idempotency_ttl_override: Option<Duration>,
}

impl Default for NanoToSwarmPolicy {
    fn default() -> Self {
        Self::experimental_fast()
    }
}

impl std::fmt::Debug for NanoToSwarmPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NanoToSwarmPolicy")
            .field("verify_signature", &self.verify_signature)
            .field("require_gold_binding", &self.require_gold_binding)
            .field("require_publisher_status", &self.require_publisher_status)
            .field("enforce_limits", &self.enforce_limits)
            .field("idempotency_cache", &self.idempotency_cache.is_some())
            .field("signature_gate", &self.signature_gate.is_some())
            .field("gold_gate", &self.gold_gate.is_some())
            .field(
                "publisher_status_gate",
                &self.publisher_status_gate.is_some(),
            )
            .field("idempotency_ttl_override", &self.idempotency_ttl_override)
            .finish()
    }
}

impl NanoToSwarmPolicy {
    /// **N23 + Annex G default** : all gates off ; raw dispatch
    /// plus envelope only.  Use for TierExperimental agents where
    /// the audit overhead is intentionally waived.
    ///
    /// Equivalent to [`Self::default`].
    pub fn experimental_fast() -> Self {
        Self {
            verify_signature: false,
            require_gold_binding: false,
            require_publisher_status: false,
            enforce_limits: true,
            idempotency_cache: None,
            signature_gate: None,
            gold_gate: None,
            publisher_status_gate: None,
            idempotency_ttl_override: None,
        }
    }

    /// **N23 + Annex G default** : all gates wired ; hard
    /// failure when any precondition is missing.  Use for Tier1+
    /// EFE built-in agents.
    pub fn tier1_strict(
        signature_gate: Arc<dyn SignatureGate>,
        gold_gate: Arc<dyn GoldBindingGate>,
        idempotency_cache: Arc<dyn IdempotencyCache>,
    ) -> Self {
        Self {
            verify_signature: true,
            require_gold_binding: true,
            require_publisher_status: false,
            enforce_limits: true,
            idempotency_cache: Some(idempotency_cache),
            signature_gate: Some(signature_gate),
            gold_gate: Some(gold_gate),
            publisher_status_gate: None,
            idempotency_ttl_override: None,
        }
    }

    /// **N24 + Annex A2** : full wired Tier-1 policy — signature
    /// plus gold plus publisher-status plus cache.  This is the
    /// preset that the gateway's default DAG uses for the
    /// `Tier1`, `Tier2`, and `TierAudit` catalogue entries.
    pub fn tier1_strict_wired(
        signature_gate: Arc<dyn SignatureGate>,
        gold_gate: Arc<dyn GoldBindingGate>,
        publisher_status_gate: Arc<dyn PublisherStatusGate>,
        idempotency_cache: Arc<dyn IdempotencyCache>,
    ) -> Self {
        Self {
            verify_signature: true,
            require_gold_binding: true,
            require_publisher_status: true,
            enforce_limits: true,
            idempotency_cache: Some(idempotency_cache),
            signature_gate: Some(signature_gate),
            gold_gate: Some(gold_gate),
            publisher_status_gate: Some(publisher_status_gate),
            idempotency_ttl_override: None,
        }
    }

    /// **N23** : intermediate policy — caching on, no compliance.
    /// Useful for performance-bench builds.
    pub fn cache_only(cache: Arc<dyn IdempotencyCache>) -> Self {
        Self {
            verify_signature: false,
            require_gold_binding: false,
            require_publisher_status: false,
            enforce_limits: true,
            idempotency_cache: Some(cache),
            signature_gate: None,
            gold_gate: None,
            publisher_status_gate: None,
            idempotency_ttl_override: None,
        }
    }

    /// **N23** : derive the effective TTL for an entry — explicit
    /// override wins, else falls back to the catalogue policy
    /// (`Auto` / `Manual`), else 60 s default.
    pub fn effective_ttl(&self, nano: &NanoAgent) -> Duration {
        if let Some(d) = self.idempotency_ttl_override {
            return d;
        }
        use efe_nano_sdk::IdempotencyPolicy;
        match nano.idempotency {
            IdempotencyPolicy::Auto { ttl_sec } | IdempotencyPolicy::Manual { ttl_sec } => {
                Duration::from_secs(ttl_sec.max(1) as u64)
            }
            IdempotencyPolicy::Disabled => Duration::from_secs(60),
        }
    }

    /// **N23** : true if the catalogue's `IdempotencyPolicy` is
    /// `Disabled` — the adapter skips cache lookup + insert even
    /// when a cache is configured.
    pub fn caching_disabled_for(&self, nano: &NanoAgent) -> bool {
        use efe_nano_sdk::IdempotencyPolicy;
        matches!(nano.idempotency, IdempotencyPolicy::Disabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{
        traits::runtime::TraceId, Author, Category, DeterminismLevel, ExecutionMode,
        IdempotencyPolicy, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
    };

    fn sample(idempotency: IdempotencyPolicy) -> NanoAgent {
        NanoAgent {
            id: NanoAgentId::new("test_agent").unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "x".into(),
            author: Author::Efe,
            category: Category::Extraction,
            stability_tier: StabilityTier::Tier1,
            execution_mode: ExecutionMode::PureRust {
                entry: "test::run".into(),
            },
            determinism: DeterminismLevel::FullDeterministic,
            idempotency,
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
            gold_set_ref: None,
            created_at: chrono::Utc::now(),
        }
    }

    fn fake_response() -> NanoAgentResponse {
        NanoAgentResponse {
            output: serde_json::json!({"ok": true}),
            latency_ms: 1,
            cost_cents: 0.0,
            trace_id: TraceId::new(),
            cache_hit: false,
        }
    }

    // ─── compute_cache_key ─────────────────────────────────────

    #[test]
    fn cache_key_stable_for_same_input() {
        let a = sample(IdempotencyPolicy::auto_24h());
        let i = serde_json::json!({"text": "hello"});
        let k1 = compute_cache_key(&a, &i);
        let k2 = compute_cache_key(&a, &i);
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 64); // SHA-256 hex.
    }

    #[test]
    fn cache_key_changes_with_version() {
        let mut a = sample(IdempotencyPolicy::auto_24h());
        let i = serde_json::json!({"text": "hello"});
        let k1 = compute_cache_key(&a, &i);
        a.version = NanoAgentVersion::new("2.0.0").unwrap();
        let k2 = compute_cache_key(&a, &i);
        assert_ne!(k1, k2);
    }

    #[test]
    fn cache_key_changes_with_input() {
        let a = sample(IdempotencyPolicy::auto_24h());
        let i1 = serde_json::json!({"text": "hello"});
        let i2 = serde_json::json!({"text": "world"});
        assert_ne!(compute_cache_key(&a, &i1), compute_cache_key(&a, &i2));
    }

    #[test]
    fn cache_key_changes_with_agent_id() {
        let mut a = sample(IdempotencyPolicy::auto_24h());
        let i = serde_json::json!({"text": "hello"});
        let k1 = compute_cache_key(&a, &i);
        a.id = NanoAgentId::new("other_agent").unwrap();
        let k2 = compute_cache_key(&a, &i);
        assert_ne!(k1, k2);
    }

    // ─── InMemoryIdempotencyCache ──────────────────────────────

    #[tokio::test]
    async fn in_mem_cache_get_miss_returns_none() {
        let c = InMemoryIdempotencyCache::new();
        let r = c.get("none").await.unwrap();
        assert!(r.is_none());
        assert!(c.is_empty());
    }

    #[tokio::test]
    async fn in_mem_cache_put_then_get_hits() {
        let c = InMemoryIdempotencyCache::new();
        c.put("k", fake_response(), Duration::from_secs(60))
            .await
            .unwrap();
        let r = c.get("k").await.unwrap();
        assert!(r.is_some());
        assert_eq!(c.len(), 1);
    }

    #[tokio::test]
    async fn in_mem_cache_expired_entry_returns_none_and_evicts() {
        let c = InMemoryIdempotencyCache::new();
        c.put("k", fake_response(), Duration::from_millis(1))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let r = c.get("k").await.unwrap();
        assert!(r.is_none());
        assert!(c.is_empty());
    }

    #[tokio::test]
    async fn in_mem_cache_overwrite_on_same_key() {
        let c = InMemoryIdempotencyCache::new();
        c.put("k", fake_response(), Duration::from_secs(60))
            .await
            .unwrap();
        let mut r2 = fake_response();
        r2.output = serde_json::json!({"different": true});
        c.put("k", r2.clone(), Duration::from_secs(60))
            .await
            .unwrap();
        let got = c.get("k").await.unwrap().unwrap();
        assert_eq!(got.output, serde_json::json!({"different": true}));
        assert_eq!(c.len(), 1);
    }

    // ─── Policy presets ────────────────────────────────────────

    #[test]
    fn experimental_fast_has_all_gates_off() {
        let p = NanoToSwarmPolicy::experimental_fast();
        assert!(!p.verify_signature);
        assert!(!p.require_gold_binding);
        assert!(p.enforce_limits);
        assert!(p.signature_gate.is_none());
        assert!(p.gold_gate.is_none());
        assert!(p.idempotency_cache.is_none());
    }

    #[test]
    fn default_equals_experimental_fast_shape() {
        let p = NanoToSwarmPolicy::default();
        assert!(!p.verify_signature);
        assert!(!p.require_gold_binding);
    }

    #[test]
    fn cache_only_has_only_cache() {
        let c = Arc::new(InMemoryIdempotencyCache::new());
        let p = NanoToSwarmPolicy::cache_only(c);
        assert!(p.idempotency_cache.is_some());
        assert!(p.signature_gate.is_none());
        assert!(p.gold_gate.is_none());
    }

    #[test]
    fn effective_ttl_uses_override_when_present() {
        let mut p = NanoToSwarmPolicy::experimental_fast();
        p.idempotency_ttl_override = Some(Duration::from_secs(7));
        let a = sample(IdempotencyPolicy::auto_24h());
        assert_eq!(p.effective_ttl(&a), Duration::from_secs(7));
    }

    #[test]
    fn effective_ttl_falls_back_to_catalogue_auto() {
        let p = NanoToSwarmPolicy::experimental_fast();
        let a = sample(IdempotencyPolicy::Auto { ttl_sec: 600 });
        assert_eq!(p.effective_ttl(&a), Duration::from_secs(600));
    }

    #[test]
    fn effective_ttl_falls_back_to_60s_when_disabled() {
        let p = NanoToSwarmPolicy::experimental_fast();
        let a = sample(IdempotencyPolicy::Disabled);
        assert_eq!(p.effective_ttl(&a), Duration::from_secs(60));
    }

    #[test]
    fn caching_disabled_for_disabled_policy() {
        let p = NanoToSwarmPolicy::experimental_fast();
        let a = sample(IdempotencyPolicy::Disabled);
        assert!(p.caching_disabled_for(&a));
        let b = sample(IdempotencyPolicy::auto_24h());
        assert!(!p.caching_disabled_for(&b));
    }

    #[test]
    fn policy_debug_does_not_panic() {
        let p = NanoToSwarmPolicy::experimental_fast();
        let _ = format!("{p:?}");
    }

    // ─── trait object compile probes ───────────────────────────

    struct AlwaysOkSig;
    #[async_trait]
    impl SignatureGate for AlwaysOkSig {
        async fn verify(
            &self,
            _nano: &NanoAgent,
            _sig: &PublisherSignature,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    struct AlwaysOkGold;
    #[async_trait]
    impl GoldBindingGate for AlwaysOkGold {
        async fn assert_approved(&self, _nano: &NanoAgent) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn signature_gate_is_object_safe() {
        let _: Arc<dyn SignatureGate> = Arc::new(AlwaysOkSig);
    }

    #[test]
    fn gold_gate_is_object_safe() {
        let _: Arc<dyn GoldBindingGate> = Arc::new(AlwaysOkGold);
    }
}
