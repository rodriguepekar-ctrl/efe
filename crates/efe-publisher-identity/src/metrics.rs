// SPDX-License-Identifier: UNLICENSED
//! **Sprint 10 — N18** : Prometheus surface for publisher identity.
//!
//! Architecture § 7.7.  12 metrics in the `n18_*` namespace.
//!
//! All counters are `AtomicU64` ; histograms are encoded as
//! `sum_us + count` pairs (microsecond fixed-point for latency) or
//! `sum + count` pairs (raw integers for cascade-size).
//!
//! Cardinality control (Architecture I20) :
//! - `tier` is one of `tier_0`/`tier_1`/`tier_2`/`tier_3` (4 values).
//! - `status` is one of `active`/`sanctions_check_stale`/`suspended`/`revoked` (4 values).
//! - `outcome` is `success`/`failure` or `clear`/`flagged`/`inconclusive` per surface.
//! - `algorithm` is `ed25519` (1 value Sprint 10).
//! - `reason` is a small closed taxonomy from `RevocationReason`.
//!
//! Operator-attributable cardinality is therefore bounded ≤ 64 per
//! surface, well under the F2 alert threshold of 10 000 lines.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;

/// **Sprint 10 — N18** : the 12 Prometheus surfaces.
pub struct N18Metrics {
    /// (1) `n18_publishers_total{tier, status}` — gauge-style counter
    /// of registered publishers.  Operator increments on registration ;
    /// labels move when status / tier changes.
    publishers: DashMap<(String, String), AtomicU64>,
    /// (2) `n18_publish_actions_total{tier, outcome}` — publish
    /// attempts ; outcome ∈ {accepted, rejected, signature_failed,
    /// publisher_revoked, publisher_suspended, cooldown_blocked,
    /// tier_insufficient}.
    publish_actions: DashMap<(String, String), AtomicU64>,
    /// (3) `n18_signature_verify_total{algorithm, outcome}` — outcome
    /// ∈ {verified, malformed, rejected}.
    signature_verify: DashMap<(String, String), AtomicU64>,
    /// (4) `n18_revocations_total{reason}` — reason ∈ {key_compromise,
    /// publisher_offboarded, sanctions_match, policy_violation,
    /// operator_action}.
    revocations: DashMap<String, AtomicU64>,
    /// (5) `n18_sanctions_checks_total{verdict}` — verdict ∈ {clear,
    /// flagged, inconclusive}.
    sanctions_checks: DashMap<String, AtomicU64>,
    /// (6) `n18_kyc_verifications_total{target_tier, outcome}` —
    /// outcome ∈ {passed, pending, failed}.
    kyc_verifications: DashMap<(String, String), AtomicU64>,
    /// (7) `n18_cooldown_blocks_total{tier}` — Tier 2+ promotion
    /// attempts blocked by the cooldown window.
    cooldown_blocks: DashMap<String, AtomicU64>,
    /// (8) `n18_co_publishing_threshold_misses_total` — N-of-M
    /// signature threshold misses (Sprint 11 runtime ; counter exists
    /// from Sprint 10 to keep the dashboard contract stable).
    co_publishing_misses: AtomicU64,
    /// (9) `n18_ledger_chain_verifies_total{outcome}` — outcome ∈
    /// {pass, fail}.
    ledger_chain_verifies: DashMap<String, AtomicU64>,
    /// (10) `n18_key_rotations_total{algorithm}` — key-rotation
    /// events per algorithm.
    key_rotations: DashMap<String, AtomicU64>,
    /// (11) `n18_publisher_lookup_latency_seconds` — summary
    /// (sum + count) of registry lookup latency.
    lookup_latency_sum_us: AtomicU64,
    lookup_latency_count: AtomicU64,
    /// (12) `n18_revocation_cascade_size{reason}` — summary
    /// (sum + count) of cascade payload size in versions revoked.
    cascade_size_sum: DashMap<String, AtomicU64>,
    cascade_size_count: DashMap<String, AtomicU64>,
}

impl std::fmt::Debug for N18Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("N18Metrics")
            .field("publishers_label_cardinality", &self.publishers.len())
            .field(
                "co_publishing_misses",
                &self.co_publishing_misses.load(Ordering::Relaxed),
            )
            .field(
                "lookup_latency_count",
                &self.lookup_latency_count.load(Ordering::Relaxed),
            )
            .finish()
    }
}

impl Default for N18Metrics {
    fn default() -> Self {
        Self::zeroed()
    }
}

impl N18Metrics {
    /// **N18** : Construct a fresh metrics surface (wrapped in `Arc`
    /// because the registry, ledger, verifier and dispatcher all hold
    /// shared references).
    pub fn new() -> Arc<Self> {
        Arc::new(Self::zeroed())
    }

    fn zeroed() -> Self {
        Self {
            publishers: DashMap::new(),
            publish_actions: DashMap::new(),
            signature_verify: DashMap::new(),
            revocations: DashMap::new(),
            sanctions_checks: DashMap::new(),
            kyc_verifications: DashMap::new(),
            cooldown_blocks: DashMap::new(),
            co_publishing_misses: AtomicU64::new(0),
            ledger_chain_verifies: DashMap::new(),
            key_rotations: DashMap::new(),
            lookup_latency_sum_us: AtomicU64::new(0),
            lookup_latency_count: AtomicU64::new(0),
            cascade_size_sum: DashMap::new(),
            cascade_size_count: DashMap::new(),
        }
    }

    fn bump_dash(map: &DashMap<String, AtomicU64>, key: &str, delta: u64) {
        map.entry(key.to_string())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(delta, Ordering::Relaxed);
    }

    fn bump_dash_pair(
        map: &DashMap<(String, String), AtomicU64>,
        k1: &str,
        k2: &str,
        delta: u64,
    ) {
        let key = (k1.to_string(), k2.to_string());
        map.entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(delta, Ordering::Relaxed);
    }

    fn dash_load(map: &DashMap<String, AtomicU64>, key: &str) -> u64 {
        map.get(key).map_or(0, |r| r.value().load(Ordering::Relaxed))
    }

    fn dash_pair_load(
        map: &DashMap<(String, String), AtomicU64>,
        k1: &str,
        k2: &str,
    ) -> u64 {
        let key = (k1.to_string(), k2.to_string());
        map.get(&key)
            .map_or(0, |r| r.value().load(Ordering::Relaxed))
    }

    // ─── (1) publishers ───────────────────────────────────────────────

    /// (1) Tick a publisher entering `(tier, status)`.
    pub fn tick_publisher(&self, tier: &str, status: &str) {
        Self::bump_dash_pair(&self.publishers, tier, status, 1);
    }

    pub fn publishers_total(&self, tier: &str, status: &str) -> u64 {
        Self::dash_pair_load(&self.publishers, tier, status)
    }

    // ─── (2) publish_actions ──────────────────────────────────────────

    /// (2) Tick a publish-action outcome.
    pub fn tick_publish_action(&self, tier: &str, outcome: &str) {
        Self::bump_dash_pair(&self.publish_actions, tier, outcome, 1);
    }

    pub fn publish_actions_total(&self, tier: &str, outcome: &str) -> u64 {
        Self::dash_pair_load(&self.publish_actions, tier, outcome)
    }

    // ─── (3) signature_verify ─────────────────────────────────────────

    /// (3) Tick a signature-verify outcome.
    pub fn tick_signature_verify(&self, algorithm: &str, outcome: &str) {
        Self::bump_dash_pair(&self.signature_verify, algorithm, outcome, 1);
    }

    pub fn signature_verify_total(&self, algorithm: &str, outcome: &str) -> u64 {
        Self::dash_pair_load(&self.signature_verify, algorithm, outcome)
    }

    // ─── (4) revocations ──────────────────────────────────────────────

    /// (4) Tick a revocation event.
    pub fn tick_revocation(&self, reason: &str) {
        Self::bump_dash(&self.revocations, reason, 1);
    }

    pub fn revocations_total(&self, reason: &str) -> u64 {
        Self::dash_load(&self.revocations, reason)
    }

    // ─── (5) sanctions_checks ─────────────────────────────────────────

    /// (5) Tick a sanctions verdict.
    pub fn tick_sanctions_check(&self, verdict: &str) {
        Self::bump_dash(&self.sanctions_checks, verdict, 1);
    }

    pub fn sanctions_checks_total(&self, verdict: &str) -> u64 {
        Self::dash_load(&self.sanctions_checks, verdict)
    }

    // ─── (6) kyc_verifications ────────────────────────────────────────

    /// (6) Tick a KYC verification outcome.
    pub fn tick_kyc_verification(&self, target_tier: &str, outcome: &str) {
        Self::bump_dash_pair(&self.kyc_verifications, target_tier, outcome, 1);
    }

    pub fn kyc_verifications_total(&self, target_tier: &str, outcome: &str) -> u64 {
        Self::dash_pair_load(&self.kyc_verifications, target_tier, outcome)
    }

    // ─── (7) cooldown_blocks ──────────────────────────────────────────

    /// (7) Tick a cooldown block.
    pub fn tick_cooldown_block(&self, tier: &str) {
        Self::bump_dash(&self.cooldown_blocks, tier, 1);
    }

    pub fn cooldown_blocks_total(&self, tier: &str) -> u64 {
        Self::dash_load(&self.cooldown_blocks, tier)
    }

    // ─── (8) co_publishing_misses ─────────────────────────────────────

    /// (8) Tick an N-of-M threshold miss.
    pub fn tick_co_publishing_miss(&self) {
        self.co_publishing_misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn co_publishing_misses_total(&self) -> u64 {
        self.co_publishing_misses.load(Ordering::Relaxed)
    }

    // ─── (9) ledger_chain_verifies ────────────────────────────────────

    /// (9) Tick a chain-verify result.
    pub fn tick_ledger_chain_verify(&self, outcome: &str) {
        Self::bump_dash(&self.ledger_chain_verifies, outcome, 1);
    }

    pub fn ledger_chain_verifies_total(&self, outcome: &str) -> u64 {
        Self::dash_load(&self.ledger_chain_verifies, outcome)
    }

    // ─── (10) key_rotations ───────────────────────────────────────────

    /// (10) Tick a key rotation.
    pub fn tick_key_rotation(&self, algorithm: &str) {
        Self::bump_dash(&self.key_rotations, algorithm, 1);
    }

    pub fn key_rotations_total(&self, algorithm: &str) -> u64 {
        Self::dash_load(&self.key_rotations, algorithm)
    }

    // ─── (11) lookup_latency_seconds ──────────────────────────────────

    /// (11) Record a lookup-latency observation.  Non-finite or
    /// negative seconds are silently dropped (F-class invariant : a
    /// bad observation must not poison the surface).
    pub fn observe_lookup_latency(&self, seconds: f64) {
        if seconds < 0.0 || !seconds.is_finite() {
            return;
        }
        let us = (seconds * 1_000_000.0).round() as u64;
        self.lookup_latency_sum_us.fetch_add(us, Ordering::Relaxed);
        self.lookup_latency_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn lookup_latency_count(&self) -> u64 {
        self.lookup_latency_count.load(Ordering::Relaxed)
    }

    pub fn lookup_latency_sum_seconds(&self) -> f64 {
        self.lookup_latency_sum_us.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }

    // ─── (12) revocation_cascade_size ─────────────────────────────────

    /// (12) Record a cascade-size observation.  `size` is the number
    /// of catalog versions atomically revoked alongside the
    /// publisher.
    pub fn observe_revocation_cascade(&self, reason: &str, size: u64) {
        Self::bump_dash(&self.cascade_size_sum, reason, size);
        Self::bump_dash(&self.cascade_size_count, reason, 1);
    }

    pub fn revocation_cascade_count(&self, reason: &str) -> u64 {
        Self::dash_load(&self.cascade_size_count, reason)
    }

    pub fn revocation_cascade_sum(&self, reason: &str) -> u64 {
        Self::dash_load(&self.cascade_size_sum, reason)
    }

    // ─── Render ───────────────────────────────────────────────────────

    /// **N18** : Render in OpenMetrics 1.0.0 text format.
    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(4_096);

        // (1)
        out.push_str("# HELP n18_publishers_total Registered publishers by tier and status.\n");
        out.push_str("# TYPE n18_publishers_total counter\n");
        for r in self.publishers.iter() {
            let (t, s) = r.key();
            out.push_str(&format!(
                "n18_publishers_total{{tier=\"{t}\",status=\"{s}\"}} {}\n",
                r.value().load(Ordering::Relaxed),
            ));
        }

        // (2)
        out.push_str("# HELP n18_publish_actions_total Publish attempts by tier and outcome.\n");
        out.push_str("# TYPE n18_publish_actions_total counter\n");
        for r in self.publish_actions.iter() {
            let (t, o) = r.key();
            out.push_str(&format!(
                "n18_publish_actions_total{{tier=\"{t}\",outcome=\"{o}\"}} {}\n",
                r.value().load(Ordering::Relaxed),
            ));
        }

        // (3)
        out.push_str("# HELP n18_signature_verify_total Signature-verification outcomes.\n");
        out.push_str("# TYPE n18_signature_verify_total counter\n");
        for r in self.signature_verify.iter() {
            let (a, o) = r.key();
            out.push_str(&format!(
                "n18_signature_verify_total{{algorithm=\"{a}\",outcome=\"{o}\"}} {}\n",
                r.value().load(Ordering::Relaxed),
            ));
        }

        // (4)
        out.push_str("# HELP n18_revocations_total Revocations by reason.\n");
        out.push_str("# TYPE n18_revocations_total counter\n");
        for r in self.revocations.iter() {
            out.push_str(&format!(
                "n18_revocations_total{{reason=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }

        // (5)
        out.push_str("# HELP n18_sanctions_checks_total Sanctions-screening verdicts.\n");
        out.push_str("# TYPE n18_sanctions_checks_total counter\n");
        for r in self.sanctions_checks.iter() {
            out.push_str(&format!(
                "n18_sanctions_checks_total{{verdict=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }

        // (6)
        out.push_str("# HELP n18_kyc_verifications_total KYC verifications by target tier and outcome.\n");
        out.push_str("# TYPE n18_kyc_verifications_total counter\n");
        for r in self.kyc_verifications.iter() {
            let (t, o) = r.key();
            out.push_str(&format!(
                "n18_kyc_verifications_total{{target_tier=\"{t}\",outcome=\"{o}\"}} {}\n",
                r.value().load(Ordering::Relaxed),
            ));
        }

        // (7)
        out.push_str("# HELP n18_cooldown_blocks_total Tier 2+ promotions blocked by cooldown.\n");
        out.push_str("# TYPE n18_cooldown_blocks_total counter\n");
        for r in self.cooldown_blocks.iter() {
            out.push_str(&format!(
                "n18_cooldown_blocks_total{{tier=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }

        // (8)
        out.push_str("# HELP n18_co_publishing_threshold_misses_total N-of-M signature threshold misses.\n");
        out.push_str("# TYPE n18_co_publishing_threshold_misses_total counter\n");
        out.push_str(&format!(
            "n18_co_publishing_threshold_misses_total {}\n",
            self.co_publishing_misses.load(Ordering::Relaxed),
        ));

        // (9)
        out.push_str("# HELP n18_ledger_chain_verifies_total Ledger chain-verify results.\n");
        out.push_str("# TYPE n18_ledger_chain_verifies_total counter\n");
        for r in self.ledger_chain_verifies.iter() {
            out.push_str(&format!(
                "n18_ledger_chain_verifies_total{{outcome=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }

        // (10)
        out.push_str("# HELP n18_key_rotations_total Publisher key rotations by algorithm.\n");
        out.push_str("# TYPE n18_key_rotations_total counter\n");
        for r in self.key_rotations.iter() {
            out.push_str(&format!(
                "n18_key_rotations_total{{algorithm=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }

        // (11)
        out.push_str("# HELP n18_publisher_lookup_latency_seconds Registry lookup latency.\n");
        out.push_str("# TYPE n18_publisher_lookup_latency_seconds summary\n");
        out.push_str(&format!(
            "n18_publisher_lookup_latency_seconds_sum {}\n",
            self.lookup_latency_sum_us.load(Ordering::Relaxed) as f64 / 1_000_000.0,
        ));
        out.push_str(&format!(
            "n18_publisher_lookup_latency_seconds_count {}\n",
            self.lookup_latency_count.load(Ordering::Relaxed),
        ));

        // (12)
        out.push_str("# HELP n18_revocation_cascade_size Versions revoked atomically per cascade.\n");
        out.push_str("# TYPE n18_revocation_cascade_size summary\n");
        for r in self.cascade_size_sum.iter() {
            let reason = r.key();
            let sum = r.value().load(Ordering::Relaxed);
            let count = self
                .cascade_size_count
                .get(reason)
                .map_or(0, |c| c.value().load(Ordering::Relaxed));
            out.push_str(&format!(
                "n18_revocation_cascade_size_sum{{reason=\"{reason}\"}} {sum}\n",
            ));
            out.push_str(&format!(
                "n18_revocation_cascade_size_count{{reason=\"{reason}\"}} {count}\n",
            ));
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_metrics_all_zero() {
        let m = N18Metrics::new();
        assert_eq!(m.publishers_total("tier_2", "active"), 0);
        assert_eq!(m.publish_actions_total("tier_2", "accepted"), 0);
        assert_eq!(m.co_publishing_misses_total(), 0);
        assert_eq!(m.lookup_latency_count(), 0);
    }

    #[test]
    fn tick_publisher_per_tier_status() {
        let m = N18Metrics::new();
        m.tick_publisher("tier_2", "active");
        m.tick_publisher("tier_2", "active");
        m.tick_publisher("tier_2", "suspended");
        m.tick_publisher("tier_3", "active");
        assert_eq!(m.publishers_total("tier_2", "active"), 2);
        assert_eq!(m.publishers_total("tier_2", "suspended"), 1);
        assert_eq!(m.publishers_total("tier_3", "active"), 1);
    }

    #[test]
    fn tick_publish_action_per_tier_outcome() {
        let m = N18Metrics::new();
        m.tick_publish_action("tier_2", "accepted");
        m.tick_publish_action("tier_2", "accepted");
        m.tick_publish_action("tier_2", "signature_failed");
        m.tick_publish_action("tier_3", "accepted");
        assert_eq!(m.publish_actions_total("tier_2", "accepted"), 2);
        assert_eq!(m.publish_actions_total("tier_2", "signature_failed"), 1);
        assert_eq!(m.publish_actions_total("tier_3", "accepted"), 1);
    }

    #[test]
    fn signature_verify_per_algorithm_outcome() {
        let m = N18Metrics::new();
        m.tick_signature_verify("ed25519", "verified");
        m.tick_signature_verify("ed25519", "verified");
        m.tick_signature_verify("ed25519", "rejected");
        assert_eq!(m.signature_verify_total("ed25519", "verified"), 2);
        assert_eq!(m.signature_verify_total("ed25519", "rejected"), 1);
    }

    #[test]
    fn revocations_per_reason() {
        let m = N18Metrics::new();
        m.tick_revocation("key_compromise");
        m.tick_revocation("key_compromise");
        m.tick_revocation("sanctions_match");
        assert_eq!(m.revocations_total("key_compromise"), 2);
        assert_eq!(m.revocations_total("sanctions_match"), 1);
    }

    #[test]
    fn sanctions_per_verdict() {
        let m = N18Metrics::new();
        m.tick_sanctions_check("clear");
        m.tick_sanctions_check("clear");
        m.tick_sanctions_check("flagged");
        m.tick_sanctions_check("inconclusive");
        assert_eq!(m.sanctions_checks_total("clear"), 2);
        assert_eq!(m.sanctions_checks_total("flagged"), 1);
        assert_eq!(m.sanctions_checks_total("inconclusive"), 1);
    }

    #[test]
    fn kyc_per_target_tier_outcome() {
        let m = N18Metrics::new();
        m.tick_kyc_verification("tier_2", "passed");
        m.tick_kyc_verification("tier_2", "passed");
        m.tick_kyc_verification("tier_3", "failed");
        assert_eq!(m.kyc_verifications_total("tier_2", "passed"), 2);
        assert_eq!(m.kyc_verifications_total("tier_3", "failed"), 1);
    }

    #[test]
    fn cooldown_blocks_per_tier() {
        let m = N18Metrics::new();
        m.tick_cooldown_block("tier_2");
        m.tick_cooldown_block("tier_2");
        m.tick_cooldown_block("tier_3");
        assert_eq!(m.cooldown_blocks_total("tier_2"), 2);
        assert_eq!(m.cooldown_blocks_total("tier_3"), 1);
    }

    #[test]
    fn co_publishing_misses_global_counter() {
        let m = N18Metrics::new();
        m.tick_co_publishing_miss();
        m.tick_co_publishing_miss();
        m.tick_co_publishing_miss();
        assert_eq!(m.co_publishing_misses_total(), 3);
    }

    #[test]
    fn ledger_chain_verifies_pass_fail() {
        let m = N18Metrics::new();
        m.tick_ledger_chain_verify("pass");
        m.tick_ledger_chain_verify("pass");
        m.tick_ledger_chain_verify("fail");
        assert_eq!(m.ledger_chain_verifies_total("pass"), 2);
        assert_eq!(m.ledger_chain_verifies_total("fail"), 1);
    }

    #[test]
    fn key_rotations_per_algorithm() {
        let m = N18Metrics::new();
        m.tick_key_rotation("ed25519");
        m.tick_key_rotation("ed25519");
        assert_eq!(m.key_rotations_total("ed25519"), 2);
    }

    #[test]
    fn observe_lookup_latency_sums_and_counts() {
        let m = N18Metrics::new();
        m.observe_lookup_latency(0.001);
        m.observe_lookup_latency(0.002);
        m.observe_lookup_latency(0.003);
        assert_eq!(m.lookup_latency_count(), 3);
        assert!((m.lookup_latency_sum_seconds() - 0.006).abs() < 1e-6);
    }

    #[test]
    fn observe_lookup_latency_ignores_invalid() {
        let m = N18Metrics::new();
        m.observe_lookup_latency(-1.0);
        m.observe_lookup_latency(f64::NAN);
        m.observe_lookup_latency(f64::INFINITY);
        assert_eq!(m.lookup_latency_count(), 0);
        assert_eq!(m.lookup_latency_sum_seconds(), 0.0);
    }

    #[test]
    fn revocation_cascade_summarises_per_reason() {
        let m = N18Metrics::new();
        m.observe_revocation_cascade("key_compromise", 3);
        m.observe_revocation_cascade("key_compromise", 5);
        m.observe_revocation_cascade("sanctions_match", 10);
        assert_eq!(m.revocation_cascade_count("key_compromise"), 2);
        assert_eq!(m.revocation_cascade_sum("key_compromise"), 8);
        assert_eq!(m.revocation_cascade_count("sanctions_match"), 1);
        assert_eq!(m.revocation_cascade_sum("sanctions_match"), 10);
    }

    #[test]
    fn render_contains_all_12_metric_names() {
        let m = N18Metrics::new();
        // Seed at least one observation per surface so labels emit.
        m.tick_publisher("tier_2", "active");
        m.tick_publish_action("tier_2", "accepted");
        m.tick_signature_verify("ed25519", "verified");
        m.tick_revocation("key_compromise");
        m.tick_sanctions_check("clear");
        m.tick_kyc_verification("tier_2", "passed");
        m.tick_cooldown_block("tier_2");
        m.tick_co_publishing_miss();
        m.tick_ledger_chain_verify("pass");
        m.tick_key_rotation("ed25519");
        m.observe_lookup_latency(0.001);
        m.observe_revocation_cascade("key_compromise", 1);
        let out = m.render_prometheus();
        for name in [
            "n18_publishers_total",
            "n18_publish_actions_total",
            "n18_signature_verify_total",
            "n18_revocations_total",
            "n18_sanctions_checks_total",
            "n18_kyc_verifications_total",
            "n18_cooldown_blocks_total",
            "n18_co_publishing_threshold_misses_total",
            "n18_ledger_chain_verifies_total",
            "n18_key_rotations_total",
            "n18_publisher_lookup_latency_seconds",
            "n18_revocation_cascade_size",
        ] {
            assert!(out.contains(name), "missing : {name}");
        }
        assert_eq!(out.matches("# HELP").count(), 12);
        assert_eq!(out.matches("# TYPE").count(), 12);
    }

    #[test]
    fn render_emits_summary_sum_and_count() {
        let m = N18Metrics::new();
        m.observe_lookup_latency(0.005);
        m.observe_lookup_latency(0.003);
        let out = m.render_prometheus();
        assert!(out.contains("n18_publisher_lookup_latency_seconds_sum 0.008"));
        assert!(out.contains("n18_publisher_lookup_latency_seconds_count 2"));
    }

    #[test]
    fn render_cascade_emits_per_reason() {
        let m = N18Metrics::new();
        m.observe_revocation_cascade("key_compromise", 4);
        m.observe_revocation_cascade("key_compromise", 6);
        let out = m.render_prometheus();
        assert!(out.contains(
            "n18_revocation_cascade_size_sum{reason=\"key_compromise\"} 10"
        ));
        assert!(out.contains(
            "n18_revocation_cascade_size_count{reason=\"key_compromise\"} 2"
        ));
    }

    #[test]
    fn debug_format_does_not_panic() {
        let m = N18Metrics::new();
        let _ = format!("{m:?}");
    }

    #[test]
    fn arc_sharing_allows_concurrent_writers() {
        let m = N18Metrics::new();
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let m = Arc::clone(&m);
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        m.tick_publisher("tier_2", "active");
                        m.tick_signature_verify("ed25519", "verified");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(m.publishers_total("tier_2", "active"), 1_600);
        assert_eq!(m.signature_verify_total("ed25519", "verified"), 1_600);
    }
}
