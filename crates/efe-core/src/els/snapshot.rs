//! Lightweight serializable snapshot of an ELS for observability / audit.
//!
//! `ElsSnapshot` deliberately hashes the centroid (FNV-1a 64-bit) rather
//! than storing it verbatim: the payload is bounded at ~50 bytes
//! regardless of the latent dimension, suitable for high-throughput
//! telemetry streams.

use serde::{Deserialize, Serialize};

use crate::els::stats::{compute_stats, ElsStats};
use crate::els::EpistemicLatentSpace;
use crate::errors::Result;
use crate::memory::Arena;
use crate::types::SwarmId;

/// Immutable snapshot of an ELS.
///
/// Serialization-friendly (bytemuck-compatible for binary encodings,
/// serde-compatible for JSON) and free of pointers or lifetimes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ElsSnapshot {
    /// Swarm identifier.
    pub swarm_id: SwarmId,
    /// Latent dimension.
    pub dim: u32,
    /// Total agents declared by the ELS.
    pub agent_count: u32,
    /// Number of alignment anchors.
    pub n_anchors: u32,
    /// Count of agents whose mask was `1` when the snapshot was taken.
    pub n_active: u32,
    /// FNV-1a 64-bit hash of the centroid's little-endian f32 bytes.
    pub centroid_hash: u64,
    /// Population variance at snapshot time.
    pub variance: f32,
    /// Gradient-norm proxy (`sqrt(variance)` in MVP).
    pub gradient_norm: f32,
    /// Outlier ratio (fraction > 2σ from centroid).
    pub outlier_ratio: f32,
    /// True iff any NaN / Inf was observed.
    pub has_anomaly: bool,
}

impl ElsSnapshot {
    /// Build a snapshot from already-computed stats and ELS metadata.
    ///
    /// Used by callers that already hold [`ElsStats`] and want to
    /// avoid the cost of recomputing them.
    #[must_use]
    pub fn from_stats(els: &EpistemicLatentSpace, stats: &ElsStats) -> Self {
        Self {
            swarm_id: els.swarm_id(),
            dim: els.dim().get(),
            agent_count: els.agent_count(),
            n_anchors: els.anchor_count(),
            n_active: stats.n_active,
            centroid_hash: fnv1a_hash(&stats.centroid),
            variance: stats.variance,
            gradient_norm: stats.gradient_norm,
            outlier_ratio: stats.outlier_ratio,
            has_anomaly: stats.has_anomaly(),
        }
    }
}

impl EpistemicLatentSpace {
    /// Produce a snapshot suitable for telemetry.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`compute_stats`].
    pub fn snapshot(&self, arena: &Arena) -> Result<ElsSnapshot> {
        let stats = compute_stats(self, arena)?;
        Ok(ElsSnapshot::from_stats(self, &stats))
    }
}

/// FNV-1a 64-bit hash over the LE bytes of an f32 slice.
///
/// Deterministic across platforms (f32 is IEEE-754; we use `to_le_bytes`
/// to fix the byte order). Two centroids differing by a single epsilon
/// produce radically different hashes; identical centroids produce
/// identical hashes.
#[must_use]
pub fn fnv1a_hash(data: &[f32]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = FNV_OFFSET;
    for x in data {
        for b in x.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(FNV_PRIME);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_empty_is_offset() {
        let h = fnv1a_hash(&[]);
        assert_eq!(h, 0xcbf2_9ce4_8422_2325);
    }

    #[test]
    fn fnv1a_deterministic() {
        let v = vec![1.0f32, 2.0, 3.0];
        assert_eq!(fnv1a_hash(&v), fnv1a_hash(&v));
    }

    #[test]
    fn fnv1a_differs_for_different_inputs() {
        let a = vec![1.0f32, 2.0, 3.0];
        let b = vec![1.0f32, 2.0, 3.001];
        assert_ne!(fnv1a_hash(&a), fnv1a_hash(&b));
    }

    #[test]
    fn fnv1a_independent_of_allocation() {
        let a: Vec<f32> = (0..64).map(|i| i as f32 * 0.1).collect();
        let b = a.clone();
        assert_eq!(fnv1a_hash(&a), fnv1a_hash(&b));
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let s = ElsSnapshot {
            swarm_id: SwarmId::new(),
            dim: 2048,
            agent_count: 512,
            n_anchors: 2,
            n_active: 510,
            centroid_hash: 0xdead_beef_1234_5678,
            variance: 0.05,
            gradient_norm: 0.2236,
            outlier_ratio: 0.02,
            has_anomaly: false,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: ElsSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn fnv1a_input_length_changes_hash() {
        let short = vec![1.0f32, 2.0];
        let long = vec![1.0f32, 2.0, 3.0];
        assert_ne!(fnv1a_hash(&short), fnv1a_hash(&long));
    }

    #[test]
    fn fnv1a_positive_vs_negative_zero_differs_is_fine() {
        // IEEE-754: +0.0 and -0.0 have different bit patterns.
        // Our hash is content-based → they differ. This is the
        // documented behavior; check it is deterministic.
        let a = vec![0.0f32];
        let b = vec![-0.0f32];
        assert_ne!(fnv1a_hash(&a), fnv1a_hash(&b));
    }
}
