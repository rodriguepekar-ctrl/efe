//! PR #20 § 15 — deterministic seed propagation via xoshiro256+.
//!
//! Every random choice in the engine (target selection, payload
//! values, think-time jitter, ramp-up offset) goes through a sub-
//! stream derived from the root seed.  This guarantees that the same
//! `(seed, profile, target list, loadgen version)` produces bit-
//! identical wire bytes — the contractual property CI relies on.
//!
//! ## Sub-streams (architecture § 15.1)
//!
//! ```text
//! root_seed ─┬─► stream A : target selection
//!            ├─► stream B : payload generation
//!            ├─► stream C : think-time jitter
//!            └─► stream D : ramp-up jitter
//! ```
//!
//! Multi-pod mode (PR #20 § 19) derives `pod_seed = root XOR ordinal`
//! before splitting.

use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256PlusPlus;

/// Logical sub-stream identifier.  Each engine module borrows its
/// dedicated stream so no two modules step on each other's randomness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u64)]
pub enum SubStream {
    /// Weighted target selection.
    TargetSelection = 0x_5cf2_8c11_0e1e_aaaa,
    /// Payload value generation.
    Payload         = 0x_5cf2_8c11_0e1e_bbbb,
    /// Think-time / inter-arrival jitter.
    ThinkTime       = 0x_5cf2_8c11_0e1e_cccc,
    /// Ramp-up offset jitter.
    RampJitter      = 0x_5cf2_8c11_0e1e_dddd,
}

/// Seed registry — a thread-safe factory for sub-stream RNGs.
///
/// Holds the root seed ; consumers ask for a `Xoshiro256PlusPlus`
/// seeded with `XOR(root, SubStream)` so the same root + SubStream
/// always yields the same stream.
#[derive(Debug, Clone)]
pub struct SeedRegistry {
    root: u64,
    pod_ordinal: u32,
}

impl SeedRegistry {
    /// Build a registry from the user-supplied (or default) root seed.
    /// Multi-pod : the engine XORs the pod ordinal into the root before
    /// passing through here.
    #[must_use]
    pub const fn new(root_seed: u64, pod_ordinal: u32) -> Self {
        Self { root: root_seed, pod_ordinal }
    }

    /// Effective pod-level seed = `root XOR (pod_ordinal as u64)`.
    #[must_use]
    pub const fn pod_seed(&self) -> u64 {
        self.root ^ (self.pod_ordinal as u64)
    }

    /// Get an RNG for the given sub-stream.
    #[must_use]
    pub fn rng(&self, stream: SubStream) -> Xoshiro256PlusPlus {
        let s = self.pod_seed() ^ stream as u64;
        Xoshiro256PlusPlus::seed_from_u64(s)
    }

    /// Convenience : the raw root seed (used in `efe.loadgen.seed`
    /// span attribute).
    #[must_use]
    pub const fn root(&self) -> u64 {
        self.root
    }

    /// Convenience : the pod ordinal.
    #[must_use]
    pub const fn pod_ordinal(&self) -> u32 {
        self.pod_ordinal
    }
}

/// Helper : weighted random selection from a slice of weights.
/// Returns the index ; weights are unnormalised (the function
/// computes the cumulative sum on the fly).
pub fn weighted_pick<R: Rng + ?Sized>(rng: &mut R, weights: &[f64]) -> usize {
    if weights.is_empty() {
        return 0;
    }
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return 0;
    }
    let mut t = rng.gen::<f64>() * total;
    for (i, &w) in weights.iter().enumerate() {
        t -= w;
        if t <= 0.0 {
            return i;
        }
    }
    weights.len() - 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;

    #[test]
    fn same_seed_same_sequence() {
        let r1 = SeedRegistry::new(42, 0);
        let r2 = SeedRegistry::new(42, 0);
        let mut a = r1.rng(SubStream::Payload);
        let mut b = r2.rng(SubStream::Payload);
        for _ in 0..16 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn substream_streams_are_independent() {
        let r = SeedRegistry::new(42, 0);
        let mut a = r.rng(SubStream::Payload);
        let mut b = r.rng(SubStream::TargetSelection);
        // First 4 numbers must differ — extremely unlikely to collide.
        let aa: Vec<u64> = (0..4).map(|_| a.next_u64()).collect();
        let bb: Vec<u64> = (0..4).map(|_| b.next_u64()).collect();
        assert_ne!(aa, bb);
    }

    #[test]
    fn different_pod_ordinals_differ() {
        let r0 = SeedRegistry::new(42, 0);
        let r1 = SeedRegistry::new(42, 1);
        assert_ne!(r0.pod_seed(), r1.pod_seed());
    }

    #[test]
    fn weighted_pick_respects_weights() {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(123);
        let weights = [0.0, 1.0]; // always pick index 1
        for _ in 0..32 {
            assert_eq!(weighted_pick(&mut rng, &weights), 1);
        }
    }

    #[test]
    fn weighted_pick_distribution_roughly_uniform() {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(123);
        let weights = [1.0, 1.0, 1.0, 1.0];
        let mut counts = [0u32; 4];
        for _ in 0..10_000 {
            let i = weighted_pick(&mut rng, &weights);
            counts[i] += 1;
        }
        // Each bucket should have ~ 2500 ± 200.
        for c in counts {
            assert!((2300..=2700).contains(&c), "bucket count {c} out of range");
        }
    }

    #[test]
    fn weighted_pick_empty_returns_zero() {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(1);
        assert_eq!(weighted_pick(&mut rng, &[]), 0);
    }
}
