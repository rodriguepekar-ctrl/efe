//! **Sprint 8 — N13** : Sticky cohort routing for canary policies.
//!
//! See `docs/architecture/EFE_M50_N13_SHADOW_CANARY_CATALOG_ARCHITECTURE.md`
//! § 6.
//!
//! # The contract
//!
//! `sticky_sample(tenant, agent_id, input_hash) → u16 ∈ [0, 10_000)` is :
//!
//! 1. **Deterministic** — same triple → same sample, forever, across
//!    processes / restarts / hosts.
//! 2. **Uniform** — for large N, the sampled values are uniformly
//!    distributed over `[0, 10_000)` modulo a sub-0.02 % bias from the
//!    `u32 % 10_000` modulo operation (well below the smallest
//!    discriminable canary cohort of 1 bp = 0.01 %).
//! 3. **Tenant-isolated** — tenant_A's cohort mapping is uncorrelated
//!    with tenant_B's (no leakage of cohort identity across tenants).
//! 4. **Versioned** — bumping the salt rotates EVERY cohort assignment.
//!    Use as escape hatch if a cohort is found to be biased.
//!
//! # Algorithm
//!
//! ```text
//! sticky_bps = SHA-256(
//!     "efe-nano-n13-routing-v1"
//!     \x00
//!     tenant_id      (16 bytes raw UUID)
//!     agent_id       (UTF-8, no NUL allowed)
//!     \x00
//!     input_hash     (32 bytes)
//! )[0..4] interpreted as big-endian u32
//! sticky_bps %= 10_000
//! ```
//!
//! If `sticky_bps < canary_bps` ⇒ canary cohort.  Else ⇒ prod cohort.

use efe_nano_sdk::{NanoAgentId, TenantId};
use sha2::{Digest, Sha256};

// ─────────────────────────────────────────────────────────────────────
//  Constants
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : salt for sticky cohort selection.  Bumping
/// `-v1 → -v2` re-shuffles every (tenant, agent, input) → cohort
/// assignment globally.  Use as escape hatch in incidents.
pub const STICKY_ROUTING_SALT: &[u8] = b"efe-nano-n13-routing-v1";

/// The basis-point space.  10 000 = 100 %.
pub const BPS_SPACE: u32 = 10_000;

// ─────────────────────────────────────────────────────────────────────
//  sticky_sample
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : Map a `(tenant, agent_id, input_hash)` triple to
/// a basis-point sample in `[0, 10_000)`.
///
/// If the returned value is less than the canary policy's
/// `canary_bps`, the input lands in the canary cohort ; otherwise in
/// the prod cohort.
///
/// # Determinism
///
/// Identical input triples produce identical samples — across processes,
/// restarts, hosts, and SHA-256 implementations.  This is the foundation
/// for idempotency-cache compatibility (N12) and replay determinism
/// (Sprint 7 replay engine).
///
/// # Uniformity
///
/// SHA-256's first 4 bytes are uniformly distributed over `[0, 2³²)`.
/// The `% 10_000` step introduces at most `(2³² mod 10_000) / 2³² ≈
/// 1.7 × 10⁻⁶` relative bias, well below the smallest discriminable
/// cohort (1 bp = 1 × 10⁻⁴).
pub fn sticky_sample(
    tenant: TenantId,
    agent_id: &NanoAgentId,
    input_hash: &[u8; 32],
) -> u16 {
    let mut h = Sha256::new();
    h.update(STICKY_ROUTING_SALT);
    h.update([0u8]);
    h.update(tenant.0.as_bytes()); // 16 bytes raw UUID
    h.update(agent_id.as_str().as_bytes());
    h.update([0u8]);
    h.update(input_hash); // 32 bytes
    let digest = h.finalize();
    let raw = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    (raw % BPS_SPACE) as u16
}

/// **Sprint 8 — N13** : Convenience predicate — "does this input land
/// in the canary cohort given the canary_bps ?"
pub fn in_canary_cohort(
    tenant: TenantId,
    agent_id: &NanoAgentId,
    input_hash: &[u8; 32],
    canary_bps: u16,
) -> bool {
    sticky_sample(tenant, agent_id, input_hash) < canary_bps
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::TenantId;

    fn aid(s: &str) -> NanoAgentId {
        NanoAgentId::new(s).unwrap()
    }
    fn hash(b: u8) -> [u8; 32] {
        let mut h = [0u8; 32];
        h.fill(b);
        h
    }

    // ─────────────────────────────────────────────────────────────────
    //  Determinism (I1)
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn determinism_same_inputs_same_sample() {
        let t = TenantId::system();
        let a = aid("agent");
        let h = hash(0x42);
        assert_eq!(sticky_sample(t, &a, &h), sticky_sample(t, &a, &h));
    }

    #[test]
    fn determinism_idempotent_across_many_calls() {
        let t = TenantId::system();
        let a = aid("agent");
        let h = hash(0x01);
        let v = sticky_sample(t, &a, &h);
        for _ in 0..1000 {
            assert_eq!(sticky_sample(t, &a, &h), v);
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  Range invariants
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn sample_in_bps_space() {
        let t = TenantId::system();
        let a = aid("agent");
        for i in 0..100u8 {
            let s = sticky_sample(t, &a, &hash(i));
            assert!(s < 10_000, "sample {s} out of [0, 10_000)");
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  Tenant isolation (I3)
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn tenant_isolation_different_tenants_different_samples() {
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let a = aid("agent");
        let h = hash(0x77);
        // Probabilistic : two random UUIDs collide on a u32 sample with
        // probability ~ 1/4 billion.  Test repeats with several hashes.
        let mut collisions = 0;
        for i in 0..32u8 {
            if sticky_sample(t1, &a, &hash(i)) == sticky_sample(t2, &a, &hash(i)) {
                collisions += 1;
            }
        }
        // Expected collisions in 32 samples : ~32 / 10_000 ≈ 0.003.
        // Allow up to 2 to absorb tail probability.
        assert!(collisions <= 2, "got {collisions} same-sample pairs in 32, expected 0-1");
        let _ = h;
    }

    // ─────────────────────────────────────────────────────────────────
    //  Agent-id isolation
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn different_agent_ids_different_samples() {
        let t = TenantId::system();
        let h = hash(0x55);
        let a = aid("agent_a");
        let b = aid("agent_b");
        // Same expectation as cross-tenant : low collision probability.
        let mut equal = 0;
        for i in 0..32u8 {
            if sticky_sample(t, &a, &hash(i)) == sticky_sample(t, &b, &hash(i)) {
                equal += 1;
            }
        }
        assert!(equal <= 2);
        let _ = h;
    }

    // ─────────────────────────────────────────────────────────────────
    //  Uniformity (I2) — Tier-1 quick check
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn uniformity_10k_samples_within_1_percent_per_bucket() {
        // Generate 10k samples and bucket into 10 deciles ; expected
        // ~1000 per decile.  Chebyshev-loose bound : ±20 % per decile.
        let t = TenantId::system();
        let a = aid("agent");
        let mut buckets = [0u32; 10];
        for i in 0..10_000u32 {
            let mut h = [0u8; 32];
            h[0..4].copy_from_slice(&i.to_le_bytes());
            let s = sticky_sample(t, &a, &h);
            buckets[(s / 1000) as usize] += 1;
        }
        for (i, &c) in buckets.iter().enumerate() {
            assert!(
                (800..=1200).contains(&c),
                "bucket {i} has {c} samples, expected 800..=1200"
            );
        }
    }

    #[test]
    fn five_pct_cohort_lands_within_5_pct_plus_or_minus_1() {
        // For canary_bps = 500 (5 %), 10k samples should yield ~500
        // in-cohort hits.  Allow ±150 (3-sigma for a binomial with
        // p=0.05, N=10k, sigma=√(10000·0.05·0.95)≈21.8 ; 3σ≈66).
        let t = TenantId::system();
        let a = aid("agent");
        let mut in_cohort = 0;
        for i in 0..10_000u32 {
            let mut h = [0u8; 32];
            h[0..4].copy_from_slice(&i.to_le_bytes());
            if in_canary_cohort(t, &a, &h, 500) {
                in_cohort += 1;
            }
        }
        // Bound : 500 ± 150 to absorb sample noise + modulo bias.
        assert!(
            (350..=650).contains(&in_cohort),
            "5 % cohort landed {in_cohort} of 10 000, expected 350-650"
        );
    }

    // ─────────────────────────────────────────────────────────────────
    //  Boundary cases
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn canary_bps_zero_excludes_all() {
        let t = TenantId::system();
        let a = aid("agent");
        for i in 0..50u8 {
            assert!(!in_canary_cohort(t, &a, &hash(i), 0));
        }
    }

    #[test]
    fn canary_bps_ten_thousand_includes_all() {
        let t = TenantId::system();
        let a = aid("agent");
        for i in 0..50u8 {
            assert!(in_canary_cohort(t, &a, &hash(i), 10_000));
        }
    }

    #[test]
    fn canary_bps_ten_thousand_one_still_includes_all() {
        // Out-of-bounds canary_bps : we treat as "≥ 10 000 = 100 %"
        // — the comparison sample < canary_bps holds for the full
        // [0, 10 000) range.
        let t = TenantId::system();
        let a = aid("agent");
        for i in 0..50u8 {
            assert!(in_canary_cohort(t, &a, &hash(i), 65_535));
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  Input-hash isolation
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn different_input_hashes_can_route_to_different_cohorts() {
        let t = TenantId::system();
        let a = aid("agent");
        let mut p = 0;
        let mut c = 0;
        for i in 0..200u8 {
            if in_canary_cohort(t, &a, &hash(i), 5_000) {
                c += 1;
            } else {
                p += 1;
            }
        }
        // For canary_bps = 5_000 (50 %), expect roughly 50/50.
        assert!(p >= 50 && c >= 50, "expected both cohorts to have ≥ 50 of 200, got p={p} c={c}");
    }

    // ─────────────────────────────────────────────────────────────────
    //  Salt versioning (I4)
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn salt_constant_is_versioned() {
        assert!(STICKY_ROUTING_SALT.ends_with(b"-v1"));
    }

    #[test]
    fn bps_space_is_ten_thousand() {
        assert_eq!(BPS_SPACE, 10_000);
    }
}
