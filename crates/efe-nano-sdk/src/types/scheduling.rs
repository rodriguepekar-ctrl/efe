//! **Sprint 8 — N15** : Fairness scheduling data model.
//!
//! See `docs/architecture/EFE_M50_N15_FAIRNESS_SCHEDULING_ARCHITECTURE.md`.
//!
//! # Module surface
//!
//! - [`TenantTier`] — billing-tier enum that drives every scheduling
//!   weight, quota, and priority across the platform.
//! - [`QuotaPolicy`] — per-tier configuration of concurrency caps,
//!   LLM rates, queue depths, audit QPS, microVM pool share.
//! - [`BackpressureError`] — typed error returned when admission is
//!   denied.  Carries `retry_after_ms` + operator-readable advice.
//! - [`BackpressureKind`] — enum discriminating the 8 denial classes.
//!
//! # Tier weights (architecture § 5.3)
//!
//! Ratios 1× / 5× / 4× / 5× between successive tiers ; total
//! Enterprise / Free = 100× under contention.  Work-conserving : at
//! low load, every tenant gets the same throughput.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::TenantId;

// ─────────────────────────────────────────────────────────────────────
//  TenantTier
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : Billing tier of a tenant.  Single knob that
/// drives every fairness-scheduling decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantTier {
    /// Trial / free tier.  Hardest quotas.  Deprioritized under
    /// platform load.  Weight 1.
    Free,
    /// Paid tier 1 — low volume.  Weight 1.
    Tier1,
    /// Paid tier 2 — medium volume.  Weight 5.
    Tier2,
    /// Paid tier 3 — high volume.  Weight 20.
    Tier3,
    /// Enterprise contract — custom SLA.  Weight 100.
    Enterprise,
    /// **Reserved** : EFE-internal services.  Highest priority,
    /// unbounded quotas.  Used by N13 shadows, M28 events, M40
    /// metrics, M24 GDPR saga — anything that must NOT be backpressured.
    Platform,
}

impl TenantTier {
    /// **Sprint 8 — N15** : weight used by `DrrScheduler` for fair-share
    /// math.  `Platform` returns `u32::MAX` (unbounded).
    pub fn drr_weight(self) -> u32 {
        match self {
            Self::Free => 1,
            Self::Tier1 => 1,
            Self::Tier2 => 5,
            Self::Tier3 => 20,
            Self::Enterprise => 100,
            Self::Platform => u32::MAX,
        }
    }

    /// Stable string discriminant for metric labels.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Tier1 => "tier1",
            Self::Tier2 => "tier2",
            Self::Tier3 => "tier3",
            Self::Enterprise => "enterprise",
            Self::Platform => "platform",
        }
    }

    /// **N15** : True if this tier has unbounded quotas (Platform only).
    pub fn is_unbounded(self) -> bool {
        matches!(self, Self::Platform)
    }
}

impl Default for TenantTier {
    fn default() -> Self {
        Self::Free
    }
}

// ─────────────────────────────────────────────────────────────────────
//  QuotaPolicy
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : per-tier quota configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaPolicy {
    /// The tier this policy applies to.
    pub tier: TenantTier,

    /// Weight used by `DrrScheduler` for slot allocation.
    pub drr_weight: u32,

    /// Max concurrent ops per tenant across all swarms.
    pub max_inflight_ops: u32,

    /// Max concurrent swarms per tenant.
    pub max_inflight_swarms: u32,

    /// Max pending requests in the per-tenant swarm intake queue.
    pub swarm_queue_depth: u32,

    /// Per-tenant LLM tokens-per-second for each provider.
    pub llm_tokens_per_second: f64,

    /// Per-tenant audit-write QPS limit.
    pub audit_writes_per_second: f64,

    /// Per-tenant Firecracker pool share in basis points
    /// (1 bp = 0.01 % of platform pool).
    pub firecracker_pool_share_bps: u16,

    /// Anti-starvation floor : maximum DRR rounds the tenant can wait
    /// for a slot before being force-granted.
    pub max_starvation_rounds: u32,
}

impl QuotaPolicy {
    /// **N15** : Free tier preset.
    pub fn free() -> Self {
        Self {
            tier: TenantTier::Free,
            drr_weight: 1,
            max_inflight_ops: 5,
            max_inflight_swarms: 1,
            swarm_queue_depth: 4,
            llm_tokens_per_second: 50.0,
            audit_writes_per_second: 50.0,
            firecracker_pool_share_bps: 100, // 1 %
            max_starvation_rounds: 10,
        }
    }

    /// **N15** : Tier 1 (entry-level paid) preset.
    pub fn tier1() -> Self {
        Self {
            tier: TenantTier::Tier1,
            drr_weight: 1,
            max_inflight_ops: 20,
            max_inflight_swarms: 5,
            swarm_queue_depth: 16,
            llm_tokens_per_second: 200.0,
            audit_writes_per_second: 200.0,
            firecracker_pool_share_bps: 250, // 2.5 %
            max_starvation_rounds: 10,
        }
    }

    /// **N15** : Tier 2 (mid-volume) preset.
    pub fn tier2() -> Self {
        Self {
            tier: TenantTier::Tier2,
            drr_weight: 5,
            max_inflight_ops: 100,
            max_inflight_swarms: 25,
            swarm_queue_depth: 64,
            llm_tokens_per_second: 1_000.0,
            audit_writes_per_second: 1_000.0,
            firecracker_pool_share_bps: 1_000, // 10 %
            max_starvation_rounds: 10,
        }
    }

    /// **N15** : Tier 3 (high-volume) preset.
    pub fn tier3() -> Self {
        Self {
            tier: TenantTier::Tier3,
            drr_weight: 20,
            max_inflight_ops: 500,
            max_inflight_swarms: 100,
            swarm_queue_depth: 256,
            llm_tokens_per_second: 5_000.0,
            audit_writes_per_second: 5_000.0,
            firecracker_pool_share_bps: 2_500, // 25 %
            max_starvation_rounds: 10,
        }
    }

    /// **N15** : Enterprise (custom SLA) preset.
    pub fn enterprise() -> Self {
        Self {
            tier: TenantTier::Enterprise,
            drr_weight: 100,
            max_inflight_ops: 2_500,
            max_inflight_swarms: 500,
            swarm_queue_depth: 1_024,
            llm_tokens_per_second: 25_000.0,
            audit_writes_per_second: 25_000.0,
            firecracker_pool_share_bps: 5_000, // 50 %
            max_starvation_rounds: 5,
        }
    }

    /// **N15** : Platform (EFE-internal) preset.  All quotas unbounded.
    pub fn platform() -> Self {
        Self {
            tier: TenantTier::Platform,
            drr_weight: u32::MAX,
            max_inflight_ops: u32::MAX,
            max_inflight_swarms: u32::MAX,
            swarm_queue_depth: u32::MAX,
            llm_tokens_per_second: f64::INFINITY,
            audit_writes_per_second: f64::INFINITY,
            firecracker_pool_share_bps: 10_000, // 100 % share allowed
            max_starvation_rounds: 0,
        }
    }

    /// **N15** : Tier → preset dispatch.
    pub fn for_tier(tier: TenantTier) -> Self {
        match tier {
            TenantTier::Free => Self::free(),
            TenantTier::Tier1 => Self::tier1(),
            TenantTier::Tier2 => Self::tier2(),
            TenantTier::Tier3 => Self::tier3(),
            TenantTier::Enterprise => Self::enterprise(),
            TenantTier::Platform => Self::platform(),
        }
    }

    /// **N15** : True if this policy has no caps (Platform tier).
    pub fn is_unbounded(&self) -> bool {
        self.tier.is_unbounded()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  BackpressureError
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : Error returned when admission is denied.
///
/// Carries `retry_after_ms` so the client can implement exponential
/// backoff, plus `advice` (operator-readable text) so client devs see
/// actionable guidance instead of an opaque error code.
///
/// **HTTP mapping** : maps to 429 Too Many Requests + `Retry-After`
/// header (integer ceiling of `retry_after_ms / 1000`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Error)]
#[error("backpressure : {kind:?} (retry after {retry_after_ms} ms) — {advice}")]
pub struct BackpressureError {
    /// Discriminator of the denial class.
    pub kind: BackpressureKind,

    /// How long the client SHOULD wait before retrying, in
    /// milliseconds.  Never zero.
    pub retry_after_ms: u32,

    /// Tenant that was denied.  Useful for ops dashboards.
    pub tenant_id: TenantId,

    /// Human-readable advice surfaced in the HTTP response body.
    pub advice: String,
}

impl BackpressureError {
    /// **N15** : construct an error with the standard advice text per
    /// kind.  Convenience for the gate / DRR / token-bucket sites that
    /// want consistent operator-visible messages.
    pub fn standard(kind: BackpressureKind, tenant_id: TenantId, retry_after_ms: u32) -> Self {
        let advice = kind.standard_advice();
        Self {
            kind,
            retry_after_ms,
            tenant_id,
            advice: advice.to_string(),
        }
    }

    /// **N15** : extend a base advice string with the retry hint.  Used
    /// by callers that want to customize the advice but keep the hint
    /// suffix consistent.
    pub fn with_advice(mut self, advice: impl Into<String>) -> Self {
        self.advice = advice.into();
        self
    }

    /// **N15** : the HTTP `Retry-After` header value (integer seconds,
    /// ceiling of `retry_after_ms / 1000`, minimum 1).
    pub fn retry_after_seconds(&self) -> u32 {
        self.retry_after_ms.div_ceil(1000).max(1)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  BackpressureKind
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : Why admission was denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackpressureKind {
    /// Tenant's max concurrent swarms reached.
    SwarmInflightFull,

    /// Tenant's pending-swarm queue is full.
    QueueFull,

    /// Tenant's max concurrent ops reached.  Either the in-flight
    /// count or the incoming request's op_count + current inflight
    /// would exceed `max_inflight_ops`.
    OpInflightFull,

    /// Platform-wide capacity reached.  All tenants affected.
    PlatformBusy,

    /// LLM provider quota exhausted for tenant.
    LlmRateLimited,

    /// Audit-write quota exhausted for tenant.
    AuditRateLimited,

    /// Firecracker microVM pool share exhausted for tenant.
    FirecrackerPoolFull,

    /// Tenant is suspended (operator action).
    TenantSuspended,

    /// **Sprint 9 — N16** : Replay rate limit hit for this tenant.
    /// Replay traffic is gated through a dedicated DRR lane capped
    /// at 10 % of the production op pool ; this kind surfaces when
    /// that lane is saturated for the tenant.
    ReplayRateLimited,
}

impl BackpressureKind {
    /// **N15** : stable string discriminant for metric labels.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SwarmInflightFull => "swarm_inflight_full",
            Self::QueueFull => "queue_full",
            Self::OpInflightFull => "op_inflight_full",
            Self::PlatformBusy => "platform_busy",
            Self::LlmRateLimited => "llm_rate_limited",
            Self::AuditRateLimited => "audit_rate_limited",
            Self::FirecrackerPoolFull => "firecracker_pool_full",
            Self::TenantSuspended => "tenant_suspended",
            Self::ReplayRateLimited => "replay_rate_limited",
        }
    }

    /// **N15** : standard operator-readable advice per kind.
    pub fn standard_advice(self) -> &'static str {
        match self {
            Self::SwarmInflightFull => {
                "Tenant max concurrent swarms reached.  Wait for one to finish, or upgrade your tier."
            }
            Self::QueueFull => {
                "Your pending-swarm queue is full.  Retry after the hint, or upgrade your tier."
            }
            Self::OpInflightFull => {
                "Tenant max concurrent ops reached.  Split into smaller swarms or upgrade your tier."
            }
            Self::PlatformBusy => {
                "Platform-wide capacity reached.  Retry after the hint (transient)."
            }
            Self::LlmRateLimited => {
                "LLM provider quota exhausted for tenant.  Retry after the hint."
            }
            Self::AuditRateLimited => {
                "Audit-write quota exhausted for tenant.  Reduce trace volume or upgrade tier."
            }
            Self::FirecrackerPoolFull => {
                "Mode B microVM pool share exhausted.  Retry after the hint."
            }
            Self::TenantSuspended => {
                "Tenant account is suspended.  Contact support."
            }
            Self::ReplayRateLimited => {
                "Replay rate limit reached.  Replays are throttled to 10 % of production capacity per tenant ; slow down operator submissions or upgrade tier."
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn tenant_tier_serde_round_trip() {
        for tier in [
            TenantTier::Free,
            TenantTier::Tier1,
            TenantTier::Tier2,
            TenantTier::Tier3,
            TenantTier::Enterprise,
            TenantTier::Platform,
        ] {
            let s = serde_json::to_string(&tier).unwrap();
            let r: TenantTier = serde_json::from_str(&s).unwrap();
            assert_eq!(tier, r);
        }
    }

    #[test]
    fn tenant_tier_weights_monotonic() {
        assert_eq!(TenantTier::Free.drr_weight(), 1);
        assert_eq!(TenantTier::Tier1.drr_weight(), 1);
        assert_eq!(TenantTier::Tier2.drr_weight(), 5);
        assert_eq!(TenantTier::Tier3.drr_weight(), 20);
        assert_eq!(TenantTier::Enterprise.drr_weight(), 100);
        // Platform : sentinel max.
        assert_eq!(TenantTier::Platform.drr_weight(), u32::MAX);
    }

    #[test]
    fn tenant_tier_as_str_stable() {
        assert_eq!(TenantTier::Free.as_str(), "free");
        assert_eq!(TenantTier::Tier1.as_str(), "tier1");
        assert_eq!(TenantTier::Tier2.as_str(), "tier2");
        assert_eq!(TenantTier::Tier3.as_str(), "tier3");
        assert_eq!(TenantTier::Enterprise.as_str(), "enterprise");
        assert_eq!(TenantTier::Platform.as_str(), "platform");
    }

    #[test]
    fn tenant_tier_is_unbounded() {
        assert!(!TenantTier::Free.is_unbounded());
        assert!(!TenantTier::Enterprise.is_unbounded());
        assert!(TenantTier::Platform.is_unbounded());
    }

    #[test]
    fn quota_policy_presets_monotonic() {
        let f = QuotaPolicy::free();
        let t1 = QuotaPolicy::tier1();
        let t2 = QuotaPolicy::tier2();
        let t3 = QuotaPolicy::tier3();
        let e = QuotaPolicy::enterprise();
        assert!(t1.max_inflight_ops > f.max_inflight_ops);
        assert!(t2.max_inflight_ops > t1.max_inflight_ops);
        assert!(t3.max_inflight_ops > t2.max_inflight_ops);
        assert!(e.max_inflight_ops > t3.max_inflight_ops);
        assert!(e.drr_weight > t3.drr_weight);
        assert!(t3.drr_weight > t2.drr_weight);
        assert!(t2.drr_weight > t1.drr_weight);
    }

    #[test]
    fn quota_policy_for_tier_dispatch() {
        assert_eq!(QuotaPolicy::for_tier(TenantTier::Free), QuotaPolicy::free());
        assert_eq!(QuotaPolicy::for_tier(TenantTier::Tier1), QuotaPolicy::tier1());
        assert_eq!(QuotaPolicy::for_tier(TenantTier::Tier2), QuotaPolicy::tier2());
        assert_eq!(QuotaPolicy::for_tier(TenantTier::Tier3), QuotaPolicy::tier3());
        assert_eq!(QuotaPolicy::for_tier(TenantTier::Enterprise), QuotaPolicy::enterprise());
        assert_eq!(QuotaPolicy::for_tier(TenantTier::Platform), QuotaPolicy::platform());
    }

    #[test]
    fn quota_policy_platform_is_unbounded() {
        let p = QuotaPolicy::platform();
        assert!(p.is_unbounded());
        assert!(p.llm_tokens_per_second.is_infinite());
        assert!(p.audit_writes_per_second.is_infinite());
        assert_eq!(p.max_inflight_ops, u32::MAX);
    }

    #[test]
    fn quota_policy_serde_round_trip() {
        for p in [
            QuotaPolicy::free(),
            QuotaPolicy::tier1(),
            QuotaPolicy::tier2(),
            QuotaPolicy::tier3(),
            QuotaPolicy::enterprise(),
        ] {
            // Platform skipped : f64::INFINITY doesn't round-trip via JSON.
            let s = serde_json::to_string(&p).unwrap();
            let r: QuotaPolicy = serde_json::from_str(&s).unwrap();
            assert_eq!(p, r);
        }
    }

    #[test]
    fn backpressure_kind_as_str_stable() {
        assert_eq!(BackpressureKind::SwarmInflightFull.as_str(), "swarm_inflight_full");
        assert_eq!(BackpressureKind::QueueFull.as_str(), "queue_full");
        assert_eq!(BackpressureKind::LlmRateLimited.as_str(), "llm_rate_limited");
        assert_eq!(BackpressureKind::TenantSuspended.as_str(), "tenant_suspended");
    }

    #[test]
    fn backpressure_kind_standard_advice_nonempty() {
        for k in [
            BackpressureKind::SwarmInflightFull,
            BackpressureKind::QueueFull,
            BackpressureKind::OpInflightFull,
            BackpressureKind::PlatformBusy,
            BackpressureKind::LlmRateLimited,
            BackpressureKind::AuditRateLimited,
            BackpressureKind::FirecrackerPoolFull,
            BackpressureKind::TenantSuspended,
        ] {
            let a = k.standard_advice();
            assert!(!a.is_empty(), "advice for {k:?} must be non-empty");
            assert!(a.len() > 10, "advice for {k:?} should be operator-readable");
        }
    }

    #[test]
    fn backpressure_error_serde_round_trip() {
        let e = BackpressureError::standard(
            BackpressureKind::QueueFull,
            TenantId::system(),
            250,
        );
        let s = serde_json::to_string(&e).unwrap();
        let r: BackpressureError = serde_json::from_str(&s).unwrap();
        assert_eq!(e, r);
    }

    #[test]
    fn backpressure_error_carries_retry_after_ms() {
        let e = BackpressureError::standard(
            BackpressureKind::PlatformBusy,
            TenantId::system(),
            500,
        );
        assert_eq!(e.retry_after_ms, 500);
        assert!(!e.advice.is_empty());
    }

    #[test]
    fn backpressure_error_retry_after_seconds_ceiling() {
        let e = BackpressureError::standard(
            BackpressureKind::QueueFull,
            TenantId::system(),
            250,
        );
        assert_eq!(e.retry_after_seconds(), 1);
        let e = BackpressureError::standard(
            BackpressureKind::QueueFull,
            TenantId::system(),
            1_500,
        );
        assert_eq!(e.retry_after_seconds(), 2);
        let e = BackpressureError::standard(
            BackpressureKind::QueueFull,
            TenantId::system(),
            1_000,
        );
        assert_eq!(e.retry_after_seconds(), 1);
    }

    #[test]
    fn backpressure_error_minimum_retry_after_one_second() {
        let e = BackpressureError::standard(
            BackpressureKind::QueueFull,
            TenantId::system(),
            0,
        );
        // Floor : never returns 0 to HTTP.
        assert_eq!(e.retry_after_seconds(), 1);
    }

    #[test]
    fn backpressure_error_with_advice_overrides() {
        let e = BackpressureError::standard(
            BackpressureKind::PlatformBusy,
            TenantId::system(),
            500,
        )
        .with_advice("Custom advice for ops");
        assert_eq!(e.advice, "Custom advice for ops");
        assert_eq!(e.retry_after_ms, 500);
    }

    #[test]
    fn quota_policy_tier_field_matches_preset() {
        assert_eq!(QuotaPolicy::free().tier, TenantTier::Free);
        assert_eq!(QuotaPolicy::tier1().tier, TenantTier::Tier1);
        assert_eq!(QuotaPolicy::tier2().tier, TenantTier::Tier2);
        assert_eq!(QuotaPolicy::tier3().tier, TenantTier::Tier3);
        assert_eq!(QuotaPolicy::enterprise().tier, TenantTier::Enterprise);
        assert_eq!(QuotaPolicy::platform().tier, TenantTier::Platform);
    }
}
