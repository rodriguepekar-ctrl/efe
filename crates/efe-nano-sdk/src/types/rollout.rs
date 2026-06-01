//! **Sprint 8 — N13** : Shadow / canary rollout policy types.
//!
//! See `docs/architecture/EFE_M50_N13_SHADOW_CANARY_CATALOG_ARCHITECTURE.md`.
//!
//! # The 3 deployment states
//!
//! Every `(tenant_id, agent_id)` tuple in the catalog has exactly ONE
//! active [`RolloutPolicy`].  The default for any agent without an
//! explicit policy is `Stable { version: <latest matching> }`.  The
//! three variants are :
//!
//! 1. [`RolloutPolicy::Stable`] — single version serves 100 % of traffic.
//! 2. [`RolloutPolicy::Shadow`] — new version runs side-by-side ; output
//!    compared but never returned to caller.  Quality-gate before
//!    canary.
//! 3. [`RolloutPolicy::Canary`] — new version serves `canary_bps`
//!    basis-points (0-10 000) of traffic, sticky-routed by input hash.
//!
//! # The 4 comparison strategies
//!
//! See [`ComparisonStrategy`] for `Exact`, `Structural`, `Semantic`,
//! `None`.  Default-by-determinism-level matrix lives in the architecture
//! doc § 7.5.
//!
//! # The cost attribution model
//!
//! [`CostAttribution`] routes shadow / canary cost cents to either the
//! `Platform` global budget or the `Tenant` per-tenant budget.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{NanoAgentId, NanoAgentVersion};

// ─────────────────────────────────────────────────────────────────────
//  RolloutPolicy
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : Per-(tenant, agent_id) deployment policy.
///
/// Stored alongside the agent in the catalog.  Atomically updatable
/// via `Catalog::set_rollout_policy`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RolloutPolicy {
    /// Steady state : a single version serves 100 % of traffic.
    Stable {
        /// The version that handles every dispatch.
        version: NanoAgentVersion,
    },

    /// Quality-gate state : production version handles 100 % of caller-
    /// visible traffic, shadow runs in parallel on every input, output
    /// compared but **never returned to the caller**.  Used to validate
    /// a new version before any traffic is shifted.
    Shadow {
        /// Version that serves the caller.
        prod_version: NanoAgentVersion,

        /// Version run in parallel — output compared, never returned.
        shadow_version: NanoAgentVersion,

        /// When the policy was applied.  Drift-stat window starts here.
        started_at: DateTime<Utc>,

        /// How to compare prod and shadow output.
        comparison: ComparisonStrategy,

        /// Where shadow cost charges.
        cost_attribution: CostAttribution,

        /// Optional per-policy shadow cost cap.  `None` ⇒ uses platform /
        /// tenant default budget per `cost_attribution`.  Explicit
        /// `Some(0.0)` ⇒ shadow off (emergency cost squeeze).
        #[serde(default)]
        shadow_budget_cents: Option<f64>,

        /// Whether to run shadow on cache hits.  Default true for
        /// shadow policies — we want full shadow coverage during the
        /// quality-gate window.
        #[serde(default = "default_true")]
        shadow_on_cache_hit: bool,
    },

    /// Rollout state : `canary_bps` basis-points (0-10 000) of traffic
    /// routed to `canary_version` ; the rest stays on `prod_version`.
    /// Routing is sticky by input hash : same input always lands on the
    /// same variant.
    Canary {
        /// Version still serving the majority.
        prod_version: NanoAgentVersion,

        /// Version receiving the canary cohort.
        canary_version: NanoAgentVersion,

        /// Cohort size in basis points (1 bp = 0.01 %, 10 000 = 100 %).
        canary_bps: u16,

        /// When the policy was applied.  Drift-stat window starts here.
        started_at: DateTime<Utc>,

        /// Gates that promotion + auto-rollback consult.
        promotion_gates: PromotionGates,

        /// Where canary cost charges.  Usually `Tenant` for canaries
        /// (they're real production traffic, billed normally).
        cost_attribution: CostAttribution,
    },
}

fn default_true() -> bool {
    true
}

impl RolloutPolicy {
    /// The version that serves the caller-visible production response.
    pub fn prod_version(&self) -> &NanoAgentVersion {
        match self {
            Self::Stable { version } => version,
            Self::Shadow { prod_version, .. } => prod_version,
            Self::Canary { prod_version, .. } => prod_version,
        }
    }

    /// The non-prod version, if any (shadow_version for Shadow,
    /// canary_version for Canary).  `None` for Stable.
    pub fn second_version(&self) -> Option<&NanoAgentVersion> {
        match self {
            Self::Stable { .. } => None,
            Self::Shadow { shadow_version, .. } => Some(shadow_version),
            Self::Canary { canary_version, .. } => Some(canary_version),
        }
    }

    /// Stable string discriminant.  Used for metric labels + log
    /// structured fields.
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Stable { .. } => "stable",
            Self::Shadow { .. } => "shadow",
            Self::Canary { .. } => "canary",
        }
    }

    /// Quick check : is the policy currently routing any traffic away
    /// from `prod_version` ?
    pub fn is_active_rollout(&self) -> bool {
        match self {
            Self::Stable { .. } => false,
            Self::Shadow { .. } => true,
            Self::Canary { canary_bps, .. } => *canary_bps > 0,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ComparisonStrategy
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : How shadow output is compared against prod
/// output.  Default-by-mode matrix in architecture doc § 7.5.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComparisonStrategy {
    /// Byte-identical output hash compared.  Default for
    /// `FullDeterministic` agents.
    Exact,

    /// JSON structural equivalence : same key set, same value types,
    /// same array lengths.  Tolerates byte-level differences within a
    /// shared shape.  Default for `LlmReplayable`.
    Structural,

    /// Semantic equivalence : both outputs passed to a comparator nano
    /// agent that returns a similarity score in `[0.0, 1.0]`.  Opt-in.
    Semantic {
        /// Id of the comparator agent.  MUST itself have a policy of
        /// `Stable` (no recursive shadowing in Sprint 8).
        comparator_agent_id: NanoAgentId,
    },

    /// No comparison performed — shadow still runs (for cost / latency
    /// analytics) but the per-call comparison is skipped.  Default for
    /// `LlmEphemeral`.
    None,
}

impl ComparisonStrategy {
    /// Stable string discriminant for metric labels.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Structural => "structural",
            Self::Semantic { .. } => "semantic",
            Self::None => "none",
        }
    }

    /// Whether comparison should be attempted at all.  `false` for
    /// `None` ; `true` for the rest.
    pub fn compares(&self) -> bool {
        !matches!(self, Self::None)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  CostAttribution
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : Where the shadow / canary cost charges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostAttribution {
    /// Platform-wide budget absorbs the cost.  Used for operator-
    /// initiated EFE-shipped agent shadowing.
    Platform,

    /// Tenant pays.  Standard for canary traffic and for tenant-
    /// authored agents that the tenant chooses to shadow.
    Tenant,
}

impl CostAttribution {
    /// Stable string discriminant for metric labels.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Platform => "platform",
            Self::Tenant => "tenant",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  PromotionGates
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : The thresholds promotion + auto-rollback
/// evaluate against the per-(tenant, agent_id) drift window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionGates {
    /// Minimum total samples before any gate is meaningful.  Default 100.
    pub min_sample_count: u32,

    /// Maximum allowed mismatch rate in basis-points (1 bp = 0.01 %).
    /// Default 100 bps = 1 %.
    pub max_drift_bps: u16,

    /// Maximum allowed canary error rate in basis-points.  Default 50
    /// bps = 0.5 %.
    pub max_error_rate_bps: u16,

    /// Maximum allowed P99 latency delta — canary may not be more than
    /// X % slower than prod.  Default 50.
    pub max_latency_p99_delta_pct: u16,

    /// Maximum allowed cost delta — canary may cost up to X % more than
    /// prod.  Default 100 (= 2× prod).
    pub max_cost_delta_pct: u16,
}

impl Default for PromotionGates {
    fn default() -> Self {
        Self {
            min_sample_count: 100,
            max_drift_bps: 100,
            max_error_rate_bps: 50,
            max_latency_p99_delta_pct: 50,
            max_cost_delta_pct: 100,
        }
    }
}

impl PromotionGates {
    /// Strict preset : appropriate for Tier 1 production-critical
    /// agents.  Half the drift tolerance, double the sample count.
    pub fn strict() -> Self {
        Self {
            min_sample_count: 1000,
            max_drift_bps: 50,
            max_error_rate_bps: 10,
            max_latency_p99_delta_pct: 25,
            max_cost_delta_pct: 50,
        }
    }

    /// Lax preset : appropriate for experimental tenant-authored
    /// agents.  More forgiving on every dimension.
    pub fn lax() -> Self {
        Self {
            min_sample_count: 50,
            max_drift_bps: 1000,
            max_error_rate_bps: 500,
            max_latency_p99_delta_pct: 200,
            max_cost_delta_pct: 500,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ShadowOutcome
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : Result of a single shadow comparison.
#[derive(Debug, Clone)]
pub enum ShadowOutcome {
    /// Production and shadow produced equivalent output (by the chosen
    /// strategy).  `similarity ∈ [0.0, 1.0]` ; `1.0` for `Exact`.
    Match {
        compared_at: DateTime<Utc>,
        similarity: f32,
        prod_latency_ms: u32,
        shadow_latency_ms: u32,
        prod_cost_cents: f64,
        shadow_cost_cents: f64,
    },

    /// Production and shadow produced different output.
    Mismatch {
        compared_at: DateTime<Utc>,
        similarity: f32,
        diff_summary: String,
        prod_latency_ms: u32,
        shadow_latency_ms: u32,
        prod_cost_cents: f64,
        shadow_cost_cents: f64,
    },

    /// Shadow itself failed (runtime error, timeout, etc.).  Comparison
    /// not possible.  Counts toward `error_rate` gate but not toward
    /// `drift_rate` gate.
    ShadowFailed {
        compared_at: DateTime<Utc>,
        error: String,
    },

    /// Comparator agent unavailable (for `Semantic` strategy).
    /// Doesn't count toward `drift_rate` or `error_rate` — just toward
    /// `sample_count` exclusion (gate evaluator skips these).
    ComparatorUnavailable {
        compared_at: DateTime<Utc>,
    },
}

impl ShadowOutcome {
    /// Stable string discriminant for metric labels + DB storage.
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Match { .. } => "match",
            Self::Mismatch { .. } => "mismatch",
            Self::ShadowFailed { .. } => "shadow_failed",
            Self::ComparatorUnavailable { .. } => "comparator_unavailable",
        }
    }

    /// When the comparison was made.
    pub fn compared_at(&self) -> DateTime<Utc> {
        match self {
            Self::Match { compared_at, .. }
            | Self::Mismatch { compared_at, .. }
            | Self::ShadowFailed { compared_at, .. }
            | Self::ComparatorUnavailable { compared_at } => *compared_at,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  DriftStats
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : Aggregated drift statistics over a window of
/// shadow outcomes.  Returned by the dispatcher's drift module +
/// surfaced via the `efe_nano_observe_canary` MCP tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriftStats {
    /// Total samples in the window (includes failed shadows + comparator-
    /// unavailable).
    pub sample_count: u32,

    /// How many `Match` outcomes.
    pub match_count: u32,

    /// How many `Mismatch` outcomes.
    pub mismatch_count: u32,

    /// How many `ShadowFailed`.
    pub failed_shadow_count: u32,

    /// How many `ComparatorUnavailable`.
    pub comparator_unavailable_count: u32,

    /// `mismatch_count / (match + mismatch)` in basis-points.
    /// 0 if no conclusive samples.
    pub mismatch_rate_bps: u16,

    /// `failed_shadow / sample_count` in basis-points.
    pub error_rate_bps: u16,

    /// `(shadow_p99 - prod_p99) / prod_p99` in percent.  Signed.
    /// 0 if insufficient samples.
    pub p99_latency_delta_pct: i32,

    /// `(shadow_cost_total - prod_cost_total) / prod_cost_total` in
    /// percent.  Signed.  0 if insufficient samples.
    pub cost_delta_pct: i32,
}

impl DriftStats {
    /// Empty stats — no samples yet.
    pub fn empty() -> Self {
        Self {
            sample_count: 0,
            match_count: 0,
            mismatch_count: 0,
            failed_shadow_count: 0,
            comparator_unavailable_count: 0,
            mismatch_rate_bps: 0,
            error_rate_bps: 0,
            p99_latency_delta_pct: 0,
            cost_delta_pct: 0,
        }
    }

    /// Conclusive sample count : matches + mismatches only.  Drift rate
    /// is computed against this denominator.
    pub fn conclusive_count(&self) -> u32 {
        self.match_count + self.mismatch_count
    }
}

// ─────────────────────────────────────────────────────────────────────
//  GateOutcome
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : Result of evaluating [`PromotionGates`] against
/// a [`DriftStats`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    /// All gates passed.  Promotion may proceed.
    Pass,

    /// One or more gates blocked.  Each `reason` describes one blocking
    /// gate (so callers can surface ALL blockers, not just the first).
    Block { reasons: Vec<String> },
}

impl GateOutcome {
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }
    pub fn is_block(&self) -> bool {
        matches!(self, Self::Block { .. })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NanoAgentId;
    use serde_json;

    fn v(s: &str) -> NanoAgentVersion {
        NanoAgentVersion::new(s).unwrap()
    }

    #[test]
    fn stable_policy_round_trip() {
        let p = RolloutPolicy::Stable { version: v("1.0.0") };
        let s = serde_json::to_string(&p).unwrap();
        let r: RolloutPolicy = serde_json::from_str(&s).unwrap();
        assert_eq!(p, r);
        assert!(s.contains("\"kind\":\"stable\""));
    }

    #[test]
    fn canary_policy_round_trip() {
        let p = RolloutPolicy::Canary {
            prod_version: v("1.0.0"),
            canary_version: v("1.1.0"),
            canary_bps: 500,
            started_at: Utc::now(),
            promotion_gates: PromotionGates::default(),
            cost_attribution: CostAttribution::Tenant,
        };
        let s = serde_json::to_string(&p).unwrap();
        let r: RolloutPolicy = serde_json::from_str(&s).unwrap();
        assert_eq!(p, r);
    }

    #[test]
    fn shadow_policy_round_trip() {
        let p = RolloutPolicy::Shadow {
            prod_version: v("1.0.0"),
            shadow_version: v("1.1.0"),
            started_at: Utc::now(),
            comparison: ComparisonStrategy::Structural,
            cost_attribution: CostAttribution::Platform,
            shadow_budget_cents: Some(50.0),
            shadow_on_cache_hit: true,
        };
        let s = serde_json::to_string(&p).unwrap();
        let r: RolloutPolicy = serde_json::from_str(&s).unwrap();
        assert_eq!(p, r);
    }

    #[test]
    fn prod_version_accessor() {
        let s = RolloutPolicy::Stable { version: v("1.0.0") };
        assert_eq!(s.prod_version(), &v("1.0.0"));
        let c = RolloutPolicy::Canary {
            prod_version: v("2.0.0"),
            canary_version: v("3.0.0"),
            canary_bps: 100,
            started_at: Utc::now(),
            promotion_gates: PromotionGates::default(),
            cost_attribution: CostAttribution::Tenant,
        };
        assert_eq!(c.prod_version(), &v("2.0.0"));
    }

    #[test]
    fn second_version_only_present_for_non_stable() {
        let s = RolloutPolicy::Stable { version: v("1.0.0") };
        assert!(s.second_version().is_none());
        let c = RolloutPolicy::Canary {
            prod_version: v("1.0.0"),
            canary_version: v("2.0.0"),
            canary_bps: 100,
            started_at: Utc::now(),
            promotion_gates: PromotionGates::default(),
            cost_attribution: CostAttribution::Tenant,
        };
        assert_eq!(c.second_version(), Some(&v("2.0.0")));
    }

    #[test]
    fn kind_str_discriminants() {
        assert_eq!(RolloutPolicy::Stable { version: v("1.0.0") }.kind_str(), "stable");
        let sh = RolloutPolicy::Shadow {
            prod_version: v("1.0.0"),
            shadow_version: v("1.1.0"),
            started_at: Utc::now(),
            comparison: ComparisonStrategy::Exact,
            cost_attribution: CostAttribution::Platform,
            shadow_budget_cents: None,
            shadow_on_cache_hit: true,
        };
        assert_eq!(sh.kind_str(), "shadow");
    }

    #[test]
    fn comparison_strategy_discriminants() {
        assert_eq!(ComparisonStrategy::Exact.as_str(), "exact");
        assert_eq!(ComparisonStrategy::Structural.as_str(), "structural");
        assert_eq!(ComparisonStrategy::None.as_str(), "none");
        let s = ComparisonStrategy::Semantic {
            comparator_agent_id: NanoAgentId::new("comparator").unwrap(),
        };
        assert_eq!(s.as_str(), "semantic");
    }

    #[test]
    fn comparison_strategy_compares_predicate() {
        assert!(ComparisonStrategy::Exact.compares());
        assert!(ComparisonStrategy::Structural.compares());
        assert!(!ComparisonStrategy::None.compares());
    }

    #[test]
    fn cost_attribution_as_str() {
        assert_eq!(CostAttribution::Platform.as_str(), "platform");
        assert_eq!(CostAttribution::Tenant.as_str(), "tenant");
    }

    #[test]
    fn promotion_gates_default_values() {
        let g = PromotionGates::default();
        assert_eq!(g.min_sample_count, 100);
        assert_eq!(g.max_drift_bps, 100);
        assert_eq!(g.max_error_rate_bps, 50);
        assert_eq!(g.max_latency_p99_delta_pct, 50);
        assert_eq!(g.max_cost_delta_pct, 100);
    }

    #[test]
    fn promotion_gates_strict_tighter_than_default() {
        let d = PromotionGates::default();
        let s = PromotionGates::strict();
        assert!(s.min_sample_count > d.min_sample_count);
        assert!(s.max_drift_bps < d.max_drift_bps);
    }

    #[test]
    fn promotion_gates_lax_more_permissive_than_default() {
        let d = PromotionGates::default();
        let l = PromotionGates::lax();
        assert!(l.max_drift_bps > d.max_drift_bps);
        assert!(l.min_sample_count < d.min_sample_count);
    }

    #[test]
    fn drift_stats_empty_zeros() {
        let s = DriftStats::empty();
        assert_eq!(s.sample_count, 0);
        assert_eq!(s.conclusive_count(), 0);
    }

    #[test]
    fn drift_stats_conclusive_count() {
        let s = DriftStats {
            sample_count: 10,
            match_count: 8,
            mismatch_count: 1,
            failed_shadow_count: 1,
            ..DriftStats::empty()
        };
        assert_eq!(s.conclusive_count(), 9);
    }

    #[test]
    fn gate_outcome_is_pass_predicate() {
        assert!(GateOutcome::Pass.is_pass());
        assert!(!GateOutcome::Pass.is_block());
        let b = GateOutcome::Block { reasons: vec!["x".into()] };
        assert!(!b.is_pass());
        assert!(b.is_block());
    }

    #[test]
    fn shadow_outcome_kind_str() {
        let now = Utc::now();
        let m = ShadowOutcome::Match {
            compared_at: now,
            similarity: 1.0,
            prod_latency_ms: 10,
            shadow_latency_ms: 12,
            prod_cost_cents: 0.001,
            shadow_cost_cents: 0.002,
        };
        assert_eq!(m.kind_str(), "match");
        assert_eq!(m.compared_at(), now);
        let f = ShadowOutcome::ShadowFailed {
            compared_at: now,
            error: "boom".into(),
        };
        assert_eq!(f.kind_str(), "shadow_failed");
    }

    #[test]
    fn shadow_policy_default_shadow_on_cache_hit_is_true() {
        // serde default applies when the field is missing.
        let json = r#"{
            "kind": "shadow",
            "prod_version": "1.0.0",
            "shadow_version": "1.1.0",
            "started_at": "2026-05-20T00:00:00Z",
            "comparison": { "kind": "exact" },
            "cost_attribution": "platform"
        }"#;
        let p: RolloutPolicy = serde_json::from_str(json).unwrap();
        match p {
            RolloutPolicy::Shadow { shadow_on_cache_hit, shadow_budget_cents, .. } => {
                assert!(shadow_on_cache_hit);
                assert!(shadow_budget_cents.is_none());
            }
            _ => panic!("expected Shadow"),
        }
    }
}
