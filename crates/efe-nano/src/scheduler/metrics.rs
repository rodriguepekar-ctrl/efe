// SPDX-License-Identifier: UNLICENSED
//! **Sprint 8 — N15** : Prometheus-compatible metrics for the
//! fairness-scheduling subsystem.
//!
//! # Surfaces (architecture § 8)
//!
//! Twelve metrics in the `n15_*` namespace :
//!
//! | # | Name                                  | Type      | Labels                       |
//! |---|---------------------------------------|-----------|------------------------------|
//! | 1 | `n15_admission_requests_total`        | counter   | `tier`                       |
//! | 2 | `n15_admission_accepted_total`        | counter   | `tier`                       |
//! | 3 | `n15_admission_rejected_total`        | counter   | `tier`, `reason`             |
//! | 4 | `n15_inflight_ops`                     | gauge     | `tier`                       |
//! | 5 | `n15_inflight_swarms`                 | gauge     | `tier`                       |
//! | 6 | `n15_queue_depth`                     | gauge     | `tier`                       |
//! | 7 | `n15_acquire_latency_seconds`         | histogram | `scheduler`                  |
//! | 8 | `n15_token_bucket_consumes_total`     | counter   | `registry`, `tier`, `outcome`|
//! | 9 | `n15_drr_rounds_total`                | counter   | `scheduler`                  |
//! |10 | `n15_starvation_force_grants_total`   | counter   | `tier`, `scheduler`          |
//! |11 | `n15_policy_lookup_total`             | counter   | `outcome`                    |
//! |12 | `n15_admission_inflight`              | gauge     | (none)                       |
//!
//! # Render format
//!
//! `SchedulerMetrics::render_prometheus()` emits OpenMetrics 1.0.0
//! text format with `# HELP` + `# TYPE` lines — directly scrapable
//! by Prometheus / Grafana / Datadog.
//!
//! # Atomic implementation
//!
//! All counters use [`AtomicU64`].  Gauges use [`AtomicI64`] (can be
//! negative briefly during concurrent decrement under heavy load).
//! Histograms use a fixed-bucket count array — no allocations on
//! the hot path.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use efe_nano_sdk::{BackpressureKind, TenantTier};

// ─────────────────────────────────────────────────────────────────────
//  Constants
// ─────────────────────────────────────────────────────────────────────

/// Number of `TenantTier` variants (used to size per-tier arrays).
pub const TIER_COUNT: usize = 6;

/// Number of `BackpressureKind` variants.
pub const KIND_COUNT: usize = 9;

/// Number of DRR scheduler instances we report on (`swarm`, `op`,
/// `firecracker`).
pub const SCHEDULER_KIND_COUNT: usize = 3;

/// Number of `TokenBucketRegistry` instances (`llm`, `audit`).
pub const REGISTRY_KIND_COUNT: usize = 2;

/// Histogram bucket boundaries (seconds).  Log-spaced from 1 µs to
/// 60 s, with a final `+Inf` overflow bucket.
const HISTOGRAM_BUCKETS_SECONDS: [f64; 16] = [
    0.000_001, 0.000_010, 0.000_050, 0.000_100, 0.000_500, 0.001, 0.005, 0.010, 0.050, 0.100,
    0.500, 1.0, 5.0, 10.0, 60.0, f64::INFINITY,
];

// ─────────────────────────────────────────────────────────────────────
//  Scheduler kind labels
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : Which DRR scheduler emitted the event.  Used
/// as a label dimension on the `scheduler` axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerKind {
    Swarm,
    Op,
    Firecracker,
}

impl SchedulerKind {
    /// Stable string for metric labels.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Swarm => "swarm",
            Self::Op => "op",
            Self::Firecracker => "firecracker",
        }
    }

    pub(crate) fn index(self) -> usize {
        match self {
            Self::Swarm => 0,
            Self::Op => 1,
            Self::Firecracker => 2,
        }
    }

    fn all() -> [SchedulerKind; SCHEDULER_KIND_COUNT] {
        [Self::Swarm, Self::Op, Self::Firecracker]
    }
}

/// **Sprint 8 — N15** : Which token bucket registry — LLM or audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryKind {
    Llm,
    Audit,
}

impl RegistryKind {
    /// Stable string for metric labels.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Llm => "llm",
            Self::Audit => "audit",
        }
    }

    pub(crate) fn index(self) -> usize {
        match self {
            Self::Llm => 0,
            Self::Audit => 1,
        }
    }

    fn all() -> [RegistryKind; REGISTRY_KIND_COUNT] {
        [Self::Llm, Self::Audit]
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tier indexing helpers
// ─────────────────────────────────────────────────────────────────────

fn tier_index(tier: TenantTier) -> usize {
    match tier {
        TenantTier::Free => 0,
        TenantTier::Tier1 => 1,
        TenantTier::Tier2 => 2,
        TenantTier::Tier3 => 3,
        TenantTier::Enterprise => 4,
        TenantTier::Platform => 5,
    }
}

fn tier_from_index(idx: usize) -> TenantTier {
    match idx {
        0 => TenantTier::Free,
        1 => TenantTier::Tier1,
        2 => TenantTier::Tier2,
        3 => TenantTier::Tier3,
        4 => TenantTier::Enterprise,
        5 => TenantTier::Platform,
        _ => TenantTier::Free,
    }
}

fn kind_index(k: BackpressureKind) -> usize {
    match k {
        BackpressureKind::SwarmInflightFull => 0,
        BackpressureKind::QueueFull => 1,
        BackpressureKind::OpInflightFull => 2,
        BackpressureKind::PlatformBusy => 3,
        BackpressureKind::LlmRateLimited => 4,
        BackpressureKind::AuditRateLimited => 5,
        BackpressureKind::FirecrackerPoolFull => 6,
        BackpressureKind::TenantSuspended => 7,
        BackpressureKind::ReplayRateLimited => 8,
    }
}

fn kind_from_index(idx: usize) -> BackpressureKind {
    match idx {
        0 => BackpressureKind::SwarmInflightFull,
        1 => BackpressureKind::QueueFull,
        2 => BackpressureKind::OpInflightFull,
        3 => BackpressureKind::PlatformBusy,
        4 => BackpressureKind::LlmRateLimited,
        5 => BackpressureKind::AuditRateLimited,
        6 => BackpressureKind::FirecrackerPoolFull,
        7 => BackpressureKind::TenantSuspended,
        8 => BackpressureKind::ReplayRateLimited,
        _ => BackpressureKind::PlatformBusy,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Histogram
// ─────────────────────────────────────────────────────────────────────

/// Fixed-bucket histogram, lock-free, suitable for hot-path latency
/// observations.
#[derive(Debug)]
pub struct Histogram {
    /// Cumulative counts per bucket (i.e. count of values ≤ bucket
    /// boundary).  Length matches [`HISTOGRAM_BUCKETS_SECONDS`].
    buckets: [AtomicU64; 16],
    /// Total observation count.
    count: AtomicU64,
    /// Cumulative sum in microseconds (avoids f64 atomic).
    sum_us: AtomicU64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            count: AtomicU64::new(0),
            sum_us: AtomicU64::new(0),
        }
    }
}

impl Histogram {
    /// Record an observation in seconds.
    pub fn observe_seconds(&self, value: f64) {
        if value.is_nan() || value < 0.0 {
            return;
        }
        let us = (value * 1_000_000.0).round() as u64;
        self.sum_us.fetch_add(us, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        for (idx, &bound) in HISTOGRAM_BUCKETS_SECONDS.iter().enumerate() {
            if value <= bound {
                self.buckets[idx].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Record an observation in microseconds.
    pub fn observe_micros(&self, micros: u64) {
        self.observe_seconds(micros as f64 / 1_000_000.0);
    }

    /// Snapshot for rendering.
    pub fn snapshot(&self) -> HistogramSnapshot {
        let mut buckets = [0u64; 16];
        for (i, b) in self.buckets.iter().enumerate() {
            buckets[i] = b.load(Ordering::Relaxed);
        }
        HistogramSnapshot {
            buckets,
            count: self.count.load(Ordering::Relaxed),
            sum_seconds: self.sum_us.load(Ordering::Relaxed) as f64 / 1_000_000.0,
        }
    }
}

/// Read-only view of a histogram.
#[derive(Debug, Clone, Copy)]
pub struct HistogramSnapshot {
    pub buckets: [u64; 16],
    pub count: u64,
    pub sum_seconds: f64,
}

// ─────────────────────────────────────────────────────────────────────
//  SchedulerMetrics
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : the 12 Prometheus surfaces.
///
/// Construction returns `Arc<Self>` so the type can be shared by the
/// gate, the dispatcher, and the metrics exporter without contention.
pub struct SchedulerMetrics {
    /// (1) `n15_admission_requests_total{tier}`
    admission_requests: [AtomicU64; TIER_COUNT],

    /// (2) `n15_admission_accepted_total{tier}`
    admission_accepted: [AtomicU64; TIER_COUNT],

    /// (3) `n15_admission_rejected_total{tier, reason}` — flat
    /// indexed by `tier * KIND_COUNT + kind`.
    admission_rejected: [AtomicU64; TIER_COUNT * KIND_COUNT],

    /// (4) `n15_inflight_ops{tier}` — current gauge.
    inflight_ops: [AtomicI64; TIER_COUNT],

    /// (5) `n15_inflight_swarms{tier}`
    inflight_swarms: [AtomicI64; TIER_COUNT],

    /// (6) `n15_queue_depth{tier}`
    queue_depth: [AtomicI64; TIER_COUNT],

    /// (7) `n15_acquire_latency_seconds{scheduler}` — three
    /// histograms, indexed by `SchedulerKind`.
    acquire_latency: [Histogram; SCHEDULER_KIND_COUNT],

    /// (8) `n15_token_bucket_consumes_total{registry, tier, outcome}`
    /// — flat indexed by `registry * TIER_COUNT * 2 + tier * 2 + outcome`.
    bucket_consumes: [AtomicU64; REGISTRY_KIND_COUNT * TIER_COUNT * 2],

    /// (9) `n15_drr_rounds_total{scheduler}`
    drr_rounds: [AtomicU64; SCHEDULER_KIND_COUNT],

    /// (10) `n15_starvation_force_grants_total{tier, scheduler}`
    /// — flat indexed by `tier * SCHEDULER_KIND_COUNT + scheduler`.
    starvation_force_grants: [AtomicU64; TIER_COUNT * SCHEDULER_KIND_COUNT],

    /// (11) `n15_policy_lookup_total{outcome}` — 2 outcomes : hit / suspended.
    policy_lookup_hit: AtomicU64,
    policy_lookup_suspended: AtomicU64,

    /// (12) `n15_admission_inflight` — total inflight gauge.
    admission_inflight: AtomicI64,
}

impl std::fmt::Debug for SchedulerMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchedulerMetrics")
            .field("admission_inflight", &self.admission_inflight.load(Ordering::Relaxed))
            .field("policy_lookup_hit", &self.policy_lookup_hit.load(Ordering::Relaxed))
            .field(
                "policy_lookup_suspended",
                &self.policy_lookup_suspended.load(Ordering::Relaxed),
            )
            .finish()
    }
}

impl SchedulerMetrics {
    /// **N15** : construct a new metrics surface, all counters at 0.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::zeroed())
    }

    /// Internal : construct a non-Arc zeroed surface (used by `new`
    /// and `Default`).
    fn zeroed() -> Self {
        Self {
            admission_requests: std::array::from_fn(|_| AtomicU64::new(0)),
            admission_accepted: std::array::from_fn(|_| AtomicU64::new(0)),
            admission_rejected: std::array::from_fn(|_| AtomicU64::new(0)),
            inflight_ops: std::array::from_fn(|_| AtomicI64::new(0)),
            inflight_swarms: std::array::from_fn(|_| AtomicI64::new(0)),
            queue_depth: std::array::from_fn(|_| AtomicI64::new(0)),
            acquire_latency: std::array::from_fn(|_| Histogram::default()),
            bucket_consumes: std::array::from_fn(|_| AtomicU64::new(0)),
            drr_rounds: std::array::from_fn(|_| AtomicU64::new(0)),
            starvation_force_grants: std::array::from_fn(|_| AtomicU64::new(0)),
            policy_lookup_hit: AtomicU64::new(0),
            policy_lookup_suspended: AtomicU64::new(0),
            admission_inflight: AtomicI64::new(0),
        }
    }

    // ─── Tick hooks ──────────────────────────────────────────────────

    /// (1) Tick an admission attempt.
    pub fn tick_admission_request(&self, tier: TenantTier) {
        self.admission_requests[tier_index(tier)].fetch_add(1, Ordering::Relaxed);
    }

    /// (2) Tick an accepted admission.
    pub fn tick_admission_accepted(&self, tier: TenantTier) {
        self.admission_accepted[tier_index(tier)].fetch_add(1, Ordering::Relaxed);
        self.admission_inflight.fetch_add(1, Ordering::Relaxed);
    }

    /// (3) Tick a rejected admission.
    pub fn tick_admission_rejected(&self, tier: TenantTier, kind: BackpressureKind) {
        let idx = tier_index(tier) * KIND_COUNT + kind_index(kind);
        self.admission_rejected[idx].fetch_add(1, Ordering::Relaxed);
    }

    /// (4-6, 12) Update the inflight / queue gauges.
    pub fn add_inflight_ops(&self, tier: TenantTier, delta: i64) {
        self.inflight_ops[tier_index(tier)].fetch_add(delta, Ordering::Relaxed);
    }
    pub fn add_inflight_swarms(&self, tier: TenantTier, delta: i64) {
        self.inflight_swarms[tier_index(tier)].fetch_add(delta, Ordering::Relaxed);
    }
    pub fn add_queue_depth(&self, tier: TenantTier, delta: i64) {
        self.queue_depth[tier_index(tier)].fetch_add(delta, Ordering::Relaxed);
    }
    pub fn add_admission_inflight(&self, delta: i64) {
        self.admission_inflight.fetch_add(delta, Ordering::Relaxed);
    }

    /// (7) Record an acquire latency.
    pub fn observe_acquire_latency(&self, scheduler: SchedulerKind, seconds: f64) {
        self.acquire_latency[scheduler.index()].observe_seconds(seconds);
    }

    /// (8) Tick a token-bucket consume.
    pub fn tick_bucket_consume(
        &self,
        registry: RegistryKind,
        tier: TenantTier,
        accepted: bool,
    ) {
        let outcome = if accepted { 0 } else { 1 };
        let idx = registry.index() * TIER_COUNT * 2 + tier_index(tier) * 2 + outcome;
        self.bucket_consumes[idx].fetch_add(1, Ordering::Relaxed);
    }

    /// (9) Tick a DRR scheduler advance round.
    pub fn tick_drr_round(&self, scheduler: SchedulerKind) {
        self.drr_rounds[scheduler.index()].fetch_add(1, Ordering::Relaxed);
    }

    /// (10) Tick an anti-starvation force-grant.
    pub fn tick_starvation_force_grant(&self, tier: TenantTier, scheduler: SchedulerKind) {
        let idx = tier_index(tier) * SCHEDULER_KIND_COUNT + scheduler.index();
        self.starvation_force_grants[idx].fetch_add(1, Ordering::Relaxed);
    }

    /// (11) Tick a policy lookup hit.
    pub fn tick_policy_lookup_hit(&self) {
        self.policy_lookup_hit.fetch_add(1, Ordering::Relaxed);
    }

    /// (11) Tick a policy lookup → suspended.
    pub fn tick_policy_lookup_suspended(&self) {
        self.policy_lookup_suspended.fetch_add(1, Ordering::Relaxed);
    }

    // ─── Snapshot / render ───────────────────────────────────────────

    /// **N15** : snapshot every metric for read-only inspection.
    pub fn snapshot(&self) -> SchedulerMetricsSnapshot {
        SchedulerMetricsSnapshot {
            admission_requests: load_arr(&self.admission_requests),
            admission_accepted: load_arr(&self.admission_accepted),
            admission_rejected: load_arr_flat(&self.admission_rejected),
            inflight_ops: load_arr_i64(&self.inflight_ops),
            inflight_swarms: load_arr_i64(&self.inflight_swarms),
            queue_depth: load_arr_i64(&self.queue_depth),
            acquire_latency: [
                self.acquire_latency[0].snapshot(),
                self.acquire_latency[1].snapshot(),
                self.acquire_latency[2].snapshot(),
            ],
            bucket_consumes: load_arr_consumes(&self.bucket_consumes),
            drr_rounds: load_arr(&self.drr_rounds),
            starvation_force_grants: load_arr_starvation(&self.starvation_force_grants),
            policy_lookup_hit: self.policy_lookup_hit.load(Ordering::Relaxed),
            policy_lookup_suspended: self.policy_lookup_suspended.load(Ordering::Relaxed),
            admission_inflight: self.admission_inflight.load(Ordering::Relaxed),
        }
    }

    /// **N15** : render every metric in OpenMetrics text format.
    pub fn render_prometheus(&self) -> String {
        let snap = self.snapshot();
        snap.render_prometheus()
    }
}

impl Default for SchedulerMetrics {
    /// Default constructor returns an un-Arc'd zeroed `Self`.  Prefer
    /// [`SchedulerMetrics::new`] for the shared-Arc form used by
    /// the gate.
    fn default() -> Self {
        Self::zeroed()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Snapshot type — value-only, suitable for JSON / Prometheus output
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : immutable view of every metric.
#[derive(Debug, Clone)]
pub struct SchedulerMetricsSnapshot {
    pub admission_requests: [u64; TIER_COUNT],
    pub admission_accepted: [u64; TIER_COUNT],
    pub admission_rejected: [u64; TIER_COUNT * KIND_COUNT],
    pub inflight_ops: [i64; TIER_COUNT],
    pub inflight_swarms: [i64; TIER_COUNT],
    pub queue_depth: [i64; TIER_COUNT],
    pub acquire_latency: [HistogramSnapshot; SCHEDULER_KIND_COUNT],
    pub bucket_consumes: [u64; REGISTRY_KIND_COUNT * TIER_COUNT * 2],
    pub drr_rounds: [u64; SCHEDULER_KIND_COUNT],
    pub starvation_force_grants: [u64; TIER_COUNT * SCHEDULER_KIND_COUNT],
    pub policy_lookup_hit: u64,
    pub policy_lookup_suspended: u64,
    pub admission_inflight: i64,
}

impl SchedulerMetricsSnapshot {
    /// Total admissions accepted across all tiers.
    pub fn total_admissions_accepted(&self) -> u64 {
        self.admission_accepted.iter().sum()
    }

    /// Total admissions rejected across all (tier, kind) pairs.
    pub fn total_admissions_rejected(&self) -> u64 {
        self.admission_rejected.iter().sum()
    }

    /// Per-kind rejection breakdown.
    pub fn rejected_by_kind(&self, kind: BackpressureKind) -> u64 {
        let k = kind_index(kind);
        (0..TIER_COUNT)
            .map(|t| self.admission_rejected[t * KIND_COUNT + k])
            .sum()
    }

    /// Per-tier rejection breakdown.
    pub fn rejected_by_tier(&self, tier: TenantTier) -> u64 {
        let t = tier_index(tier);
        (0..KIND_COUNT)
            .map(|k| self.admission_rejected[t * KIND_COUNT + k])
            .sum()
    }

    /// **N15** : DRR rounds counter for a specific scheduler.
    pub fn drr_rounds_for(&self, scheduler: SchedulerKind) -> u64 {
        self.drr_rounds[scheduler.index()]
    }

    /// **N15** : Starvation force-grant counter for `(tier, scheduler)`.
    pub fn starvation_force_grants_for(
        &self,
        tier: TenantTier,
        scheduler: SchedulerKind,
    ) -> u64 {
        let idx = tier_index(tier) * SCHEDULER_KIND_COUNT + scheduler.index();
        self.starvation_force_grants[idx]
    }

    /// **N15** : Total starvation force-grants across all tiers for
    /// a specific scheduler.
    pub fn starvation_force_grants_total(&self, scheduler: SchedulerKind) -> u64 {
        (0..TIER_COUNT)
            .map(|t| {
                self.starvation_force_grants[t * SCHEDULER_KIND_COUNT + scheduler.index()]
            })
            .sum()
    }

    /// **N15** : Inflight ops gauge for a specific tier.
    pub fn inflight_ops_for(&self, tier: TenantTier) -> i64 {
        self.inflight_ops[tier_index(tier)]
    }

    /// **N15** : Inflight swarms gauge for a specific tier.
    pub fn inflight_swarms_for(&self, tier: TenantTier) -> i64 {
        self.inflight_swarms[tier_index(tier)]
    }

    /// **N15** : Admission requests counter for a specific tier.
    pub fn admission_requests_for(&self, tier: TenantTier) -> u64 {
        self.admission_requests[tier_index(tier)]
    }

    /// **N15** : Admission accepted counter for a specific tier.
    pub fn admission_accepted_for(&self, tier: TenantTier) -> u64 {
        self.admission_accepted[tier_index(tier)]
    }

    /// **N15** : Token-bucket consume counter for `(registry, tier,
    /// outcome)`.
    pub fn bucket_consumes_for(
        &self,
        registry: RegistryKind,
        tier: TenantTier,
        accepted: bool,
    ) -> u64 {
        let outcome = if accepted { 0 } else { 1 };
        let idx = registry.index() * TIER_COUNT * 2 + tier_index(tier) * 2 + outcome;
        self.bucket_consumes[idx]
    }

    /// **N15** : Acquire-latency histogram for a specific scheduler.
    pub fn acquire_latency_for(&self, scheduler: SchedulerKind) -> HistogramSnapshot {
        self.acquire_latency[scheduler.index()]
    }

    /// **N15** : render as Prometheus 0.0.4 / OpenMetrics 1.0.0 text.
    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(4_096);

        // 1.
        out.push_str("# HELP n15_admission_requests_total Total admission attempts.\n");
        out.push_str("# TYPE n15_admission_requests_total counter\n");
        for i in 0..TIER_COUNT {
            let tier = tier_from_index(i);
            out.push_str(&format!(
                "n15_admission_requests_total{{tier=\"{}\"}} {}\n",
                tier.as_str(),
                self.admission_requests[i]
            ));
        }

        // 2.
        out.push_str("# HELP n15_admission_accepted_total Accepted admission attempts.\n");
        out.push_str("# TYPE n15_admission_accepted_total counter\n");
        for i in 0..TIER_COUNT {
            let tier = tier_from_index(i);
            out.push_str(&format!(
                "n15_admission_accepted_total{{tier=\"{}\"}} {}\n",
                tier.as_str(),
                self.admission_accepted[i]
            ));
        }

        // 3.
        out.push_str("# HELP n15_admission_rejected_total Rejected admission attempts.\n");
        out.push_str("# TYPE n15_admission_rejected_total counter\n");
        for i in 0..TIER_COUNT {
            for k in 0..KIND_COUNT {
                let tier = tier_from_index(i);
                let kind = kind_from_index(k);
                out.push_str(&format!(
                    "n15_admission_rejected_total{{tier=\"{}\",reason=\"{}\"}} {}\n",
                    tier.as_str(),
                    kind.as_str(),
                    self.admission_rejected[i * KIND_COUNT + k]
                ));
            }
        }

        // 4.
        out.push_str("# HELP n15_inflight_ops Operations in flight (per tier).\n");
        out.push_str("# TYPE n15_inflight_ops gauge\n");
        for i in 0..TIER_COUNT {
            let tier = tier_from_index(i);
            out.push_str(&format!(
                "n15_inflight_ops{{tier=\"{}\"}} {}\n",
                tier.as_str(),
                self.inflight_ops[i]
            ));
        }

        // 5.
        out.push_str("# HELP n15_inflight_swarms Swarms in flight (per tier).\n");
        out.push_str("# TYPE n15_inflight_swarms gauge\n");
        for i in 0..TIER_COUNT {
            let tier = tier_from_index(i);
            out.push_str(&format!(
                "n15_inflight_swarms{{tier=\"{}\"}} {}\n",
                tier.as_str(),
                self.inflight_swarms[i]
            ));
        }

        // 6.
        out.push_str("# HELP n15_queue_depth Swarm intake queue depth (per tier).\n");
        out.push_str("# TYPE n15_queue_depth gauge\n");
        for i in 0..TIER_COUNT {
            let tier = tier_from_index(i);
            out.push_str(&format!(
                "n15_queue_depth{{tier=\"{}\"}} {}\n",
                tier.as_str(),
                self.queue_depth[i]
            ));
        }

        // 7.
        out.push_str("# HELP n15_acquire_latency_seconds Acquire latency histogram.\n");
        out.push_str("# TYPE n15_acquire_latency_seconds histogram\n");
        for scheduler in SchedulerKind::all() {
            let h = &self.acquire_latency[scheduler.index()];
            for (i, &bound) in HISTOGRAM_BUCKETS_SECONDS.iter().enumerate() {
                let label_bound = if bound.is_infinite() {
                    String::from("+Inf")
                } else {
                    format!("{bound}")
                };
                out.push_str(&format!(
                    "n15_acquire_latency_seconds_bucket{{scheduler=\"{}\",le=\"{}\"}} {}\n",
                    scheduler.as_str(),
                    label_bound,
                    h.buckets[i]
                ));
            }
            out.push_str(&format!(
                "n15_acquire_latency_seconds_sum{{scheduler=\"{}\"}} {}\n",
                scheduler.as_str(),
                h.sum_seconds
            ));
            out.push_str(&format!(
                "n15_acquire_latency_seconds_count{{scheduler=\"{}\"}} {}\n",
                scheduler.as_str(),
                h.count
            ));
        }

        // 8.
        out.push_str(
            "# HELP n15_token_bucket_consumes_total Token bucket consume attempts.\n",
        );
        out.push_str("# TYPE n15_token_bucket_consumes_total counter\n");
        for registry in RegistryKind::all() {
            for i in 0..TIER_COUNT {
                for outcome in ["accept", "reject"] {
                    let oi = if outcome == "accept" { 0 } else { 1 };
                    let idx = registry.index() * TIER_COUNT * 2 + i * 2 + oi;
                    let tier = tier_from_index(i);
                    out.push_str(&format!(
                        "n15_token_bucket_consumes_total{{registry=\"{}\",tier=\"{}\",outcome=\"{}\"}} {}\n",
                        registry.as_str(),
                        tier.as_str(),
                        outcome,
                        self.bucket_consumes[idx]
                    ));
                }
            }
        }

        // 9.
        out.push_str("# HELP n15_drr_rounds_total DRR scheduler rounds.\n");
        out.push_str("# TYPE n15_drr_rounds_total counter\n");
        for scheduler in SchedulerKind::all() {
            out.push_str(&format!(
                "n15_drr_rounds_total{{scheduler=\"{}\"}} {}\n",
                scheduler.as_str(),
                self.drr_rounds[scheduler.index()]
            ));
        }

        // 10.
        out.push_str(
            "# HELP n15_starvation_force_grants_total Anti-starvation force-grants.\n",
        );
        out.push_str("# TYPE n15_starvation_force_grants_total counter\n");
        for i in 0..TIER_COUNT {
            for scheduler in SchedulerKind::all() {
                let tier = tier_from_index(i);
                let idx = i * SCHEDULER_KIND_COUNT + scheduler.index();
                out.push_str(&format!(
                    "n15_starvation_force_grants_total{{tier=\"{}\",scheduler=\"{}\"}} {}\n",
                    tier.as_str(),
                    scheduler.as_str(),
                    self.starvation_force_grants[idx]
                ));
            }
        }

        // 11.
        out.push_str("# HELP n15_policy_lookup_total Policy lookup outcomes.\n");
        out.push_str("# TYPE n15_policy_lookup_total counter\n");
        out.push_str(&format!(
            "n15_policy_lookup_total{{outcome=\"hit\"}} {}\n",
            self.policy_lookup_hit
        ));
        out.push_str(&format!(
            "n15_policy_lookup_total{{outcome=\"suspended\"}} {}\n",
            self.policy_lookup_suspended
        ));

        // 12.
        out.push_str("# HELP n15_admission_inflight Total admissions currently in flight.\n");
        out.push_str("# TYPE n15_admission_inflight gauge\n");
        out.push_str(&format!(
            "n15_admission_inflight {}\n",
            self.admission_inflight
        ));

        out
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Helpers
// ─────────────────────────────────────────────────────────────────────

fn load_arr<const N: usize>(arr: &[AtomicU64; N]) -> [u64; N] {
    let mut out = [0u64; N];
    for (i, a) in arr.iter().enumerate() {
        out[i] = a.load(Ordering::Relaxed);
    }
    out
}

fn load_arr_i64<const N: usize>(arr: &[AtomicI64; N]) -> [i64; N] {
    let mut out = [0i64; N];
    for (i, a) in arr.iter().enumerate() {
        out[i] = a.load(Ordering::Relaxed);
    }
    out
}

fn load_arr_flat(arr: &[AtomicU64; TIER_COUNT * KIND_COUNT]) -> [u64; TIER_COUNT * KIND_COUNT] {
    let mut out = [0u64; TIER_COUNT * KIND_COUNT];
    for (i, a) in arr.iter().enumerate() {
        out[i] = a.load(Ordering::Relaxed);
    }
    out
}

fn load_arr_consumes(
    arr: &[AtomicU64; REGISTRY_KIND_COUNT * TIER_COUNT * 2],
) -> [u64; REGISTRY_KIND_COUNT * TIER_COUNT * 2] {
    let mut out = [0u64; REGISTRY_KIND_COUNT * TIER_COUNT * 2];
    for (i, a) in arr.iter().enumerate() {
        out[i] = a.load(Ordering::Relaxed);
    }
    out
}

fn load_arr_starvation(
    arr: &[AtomicU64; TIER_COUNT * SCHEDULER_KIND_COUNT],
) -> [u64; TIER_COUNT * SCHEDULER_KIND_COUNT] {
    let mut out = [0u64; TIER_COUNT * SCHEDULER_KIND_COUNT];
    for (i, a) in arr.iter().enumerate() {
        out[i] = a.load(Ordering::Relaxed);
    }
    out
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_metrics_all_zero() {
        let m = SchedulerMetrics::new();
        let snap = m.snapshot();
        assert_eq!(snap.total_admissions_accepted(), 0);
        assert_eq!(snap.total_admissions_rejected(), 0);
        assert_eq!(snap.policy_lookup_hit, 0);
        assert_eq!(snap.admission_inflight, 0);
    }

    #[test]
    fn admission_request_ticks_per_tier() {
        let m = SchedulerMetrics::new();
        m.tick_admission_request(TenantTier::Free);
        m.tick_admission_request(TenantTier::Free);
        m.tick_admission_request(TenantTier::Tier3);
        let snap = m.snapshot();
        assert_eq!(snap.admission_requests[tier_index(TenantTier::Free)], 2);
        assert_eq!(snap.admission_requests[tier_index(TenantTier::Tier3)], 1);
    }

    #[test]
    fn admission_accepted_bumps_inflight_gauge() {
        let m = SchedulerMetrics::new();
        m.tick_admission_accepted(TenantTier::Tier1);
        m.tick_admission_accepted(TenantTier::Tier1);
        let snap = m.snapshot();
        assert_eq!(snap.admission_inflight, 2);
        assert_eq!(snap.total_admissions_accepted(), 2);
    }

    #[test]
    fn admission_rejected_breakdowns() {
        let m = SchedulerMetrics::new();
        m.tick_admission_rejected(TenantTier::Free, BackpressureKind::TenantSuspended);
        m.tick_admission_rejected(TenantTier::Free, BackpressureKind::OpInflightFull);
        m.tick_admission_rejected(TenantTier::Tier1, BackpressureKind::OpInflightFull);
        let snap = m.snapshot();
        assert_eq!(snap.total_admissions_rejected(), 3);
        assert_eq!(
            snap.rejected_by_kind(BackpressureKind::OpInflightFull),
            2
        );
        assert_eq!(snap.rejected_by_tier(TenantTier::Free), 2);
        assert_eq!(snap.rejected_by_tier(TenantTier::Tier1), 1);
    }

    #[test]
    fn inflight_gauges_handle_delta() {
        let m = SchedulerMetrics::new();
        m.add_inflight_ops(TenantTier::Tier2, 10);
        m.add_inflight_swarms(TenantTier::Tier2, 3);
        m.add_inflight_ops(TenantTier::Tier2, -2);
        let snap = m.snapshot();
        assert_eq!(snap.inflight_ops[tier_index(TenantTier::Tier2)], 8);
        assert_eq!(snap.inflight_swarms[tier_index(TenantTier::Tier2)], 3);
    }

    #[test]
    fn histogram_observes_value() {
        let h = Histogram::default();
        h.observe_seconds(0.001);
        h.observe_seconds(0.01);
        h.observe_seconds(1.0);
        let s = h.snapshot();
        assert_eq!(s.count, 3);
        assert!(s.sum_seconds > 0.0);
        // Bucket 1ms = idx 5, should have >= 1 (0.001 ≤ 0.001).
        assert!(s.buckets[5] >= 1);
        // Bucket +Inf (last) should have count = 3.
        assert_eq!(s.buckets[15], 3);
    }

    #[test]
    fn histogram_negative_or_nan_ignored() {
        let h = Histogram::default();
        h.observe_seconds(-1.0);
        h.observe_seconds(f64::NAN);
        let s = h.snapshot();
        assert_eq!(s.count, 0);
    }

    #[test]
    fn acquire_latency_observes_per_scheduler() {
        let m = SchedulerMetrics::new();
        m.observe_acquire_latency(SchedulerKind::Swarm, 0.01);
        m.observe_acquire_latency(SchedulerKind::Op, 0.005);
        let snap = m.snapshot();
        assert_eq!(snap.acquire_latency[SchedulerKind::Swarm.index()].count, 1);
        assert_eq!(snap.acquire_latency[SchedulerKind::Op.index()].count, 1);
        assert_eq!(
            snap.acquire_latency[SchedulerKind::Firecracker.index()].count,
            0
        );
    }

    #[test]
    fn token_bucket_consumes_per_dimension() {
        let m = SchedulerMetrics::new();
        m.tick_bucket_consume(RegistryKind::Llm, TenantTier::Free, true);
        m.tick_bucket_consume(RegistryKind::Llm, TenantTier::Free, false);
        m.tick_bucket_consume(RegistryKind::Audit, TenantTier::Enterprise, true);
        let snap = m.snapshot();
        // Free, LLM, accept.
        let i_accept = RegistryKind::Llm.index() * TIER_COUNT * 2
            + tier_index(TenantTier::Free) * 2;
        let i_reject = i_accept + 1;
        assert_eq!(snap.bucket_consumes[i_accept], 1);
        assert_eq!(snap.bucket_consumes[i_reject], 1);
    }

    #[test]
    fn drr_rounds_per_scheduler() {
        let m = SchedulerMetrics::new();
        m.tick_drr_round(SchedulerKind::Op);
        m.tick_drr_round(SchedulerKind::Op);
        m.tick_drr_round(SchedulerKind::Swarm);
        let snap = m.snapshot();
        assert_eq!(snap.drr_rounds[SchedulerKind::Op.index()], 2);
        assert_eq!(snap.drr_rounds[SchedulerKind::Swarm.index()], 1);
        assert_eq!(snap.drr_rounds[SchedulerKind::Firecracker.index()], 0);
    }

    #[test]
    fn starvation_force_grants_per_tier_scheduler() {
        let m = SchedulerMetrics::new();
        m.tick_starvation_force_grant(TenantTier::Free, SchedulerKind::Op);
        m.tick_starvation_force_grant(TenantTier::Free, SchedulerKind::Op);
        m.tick_starvation_force_grant(TenantTier::Tier3, SchedulerKind::Swarm);
        let snap = m.snapshot();
        let idx_free_op = tier_index(TenantTier::Free) * SCHEDULER_KIND_COUNT
            + SchedulerKind::Op.index();
        let idx_t3_swarm = tier_index(TenantTier::Tier3) * SCHEDULER_KIND_COUNT
            + SchedulerKind::Swarm.index();
        assert_eq!(snap.starvation_force_grants[idx_free_op], 2);
        assert_eq!(snap.starvation_force_grants[idx_t3_swarm], 1);
    }

    #[test]
    fn policy_lookup_counters() {
        let m = SchedulerMetrics::new();
        m.tick_policy_lookup_hit();
        m.tick_policy_lookup_hit();
        m.tick_policy_lookup_suspended();
        let snap = m.snapshot();
        assert_eq!(snap.policy_lookup_hit, 2);
        assert_eq!(snap.policy_lookup_suspended, 1);
    }

    #[test]
    fn render_prometheus_contains_all_metric_names() {
        let m = SchedulerMetrics::new();
        m.tick_admission_request(TenantTier::Free);
        m.tick_admission_accepted(TenantTier::Free);
        m.tick_admission_rejected(TenantTier::Free, BackpressureKind::OpInflightFull);
        m.observe_acquire_latency(SchedulerKind::Op, 0.005);
        m.tick_drr_round(SchedulerKind::Op);
        m.tick_starvation_force_grant(TenantTier::Free, SchedulerKind::Op);
        m.tick_policy_lookup_hit();
        m.tick_bucket_consume(RegistryKind::Llm, TenantTier::Free, true);
        let out = m.render_prometheus();
        assert!(out.contains("n15_admission_requests_total"));
        assert!(out.contains("n15_admission_accepted_total"));
        assert!(out.contains("n15_admission_rejected_total"));
        assert!(out.contains("n15_inflight_ops"));
        assert!(out.contains("n15_inflight_swarms"));
        assert!(out.contains("n15_queue_depth"));
        assert!(out.contains("n15_acquire_latency_seconds"));
        assert!(out.contains("n15_token_bucket_consumes_total"));
        assert!(out.contains("n15_drr_rounds_total"));
        assert!(out.contains("n15_starvation_force_grants_total"));
        assert!(out.contains("n15_policy_lookup_total"));
        assert!(out.contains("n15_admission_inflight"));
        // OpenMetrics structure : HELP + TYPE per metric.
        let help_count = out.matches("# HELP").count();
        let type_count = out.matches("# TYPE").count();
        assert_eq!(help_count, 12);
        assert_eq!(type_count, 12);
    }

    #[test]
    fn render_prometheus_emits_all_tier_labels() {
        let m = SchedulerMetrics::new();
        let out = m.render_prometheus();
        for tier in [
            TenantTier::Free,
            TenantTier::Tier1,
            TenantTier::Tier2,
            TenantTier::Tier3,
            TenantTier::Enterprise,
            TenantTier::Platform,
        ] {
            assert!(out.contains(&format!("tier=\"{}\"", tier.as_str())));
        }
    }

    #[test]
    fn render_prometheus_emits_kind_labels() {
        let m = SchedulerMetrics::new();
        let out = m.render_prometheus();
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
            assert!(out.contains(&format!("reason=\"{}\"", k.as_str())));
        }
    }

    #[test]
    fn debug_format() {
        let m = SchedulerMetrics::new();
        let _ = format!("{m:?}");
        let h = Histogram::default();
        let _ = format!("{h:?}");
    }

    #[test]
    fn scheduler_kind_as_str_is_stable() {
        assert_eq!(SchedulerKind::Swarm.as_str(), "swarm");
        assert_eq!(SchedulerKind::Op.as_str(), "op");
        assert_eq!(SchedulerKind::Firecracker.as_str(), "firecracker");
    }

    #[test]
    fn registry_kind_as_str_is_stable() {
        assert_eq!(RegistryKind::Llm.as_str(), "llm");
        assert_eq!(RegistryKind::Audit.as_str(), "audit");
    }
}
