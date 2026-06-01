//! Centroid update math : `μ_new = α · μ_old + (1 − α) · mean(gold)`
//! with safety drift threshold to detect adversarial poisoning.
//!
//! See § 13 of EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md.

use crate::{ActiveLearningError, ActiveLearningResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CentroidUpdateConfig {
    pub anchor_weight: f32,
    pub min_gold_examples_per_class: u32,
    pub max_centroid_drift: f32,
}

impl Default for CentroidUpdateConfig {
    fn default() -> Self {
        Self {
            anchor_weight: 0.70,
            min_gold_examples_per_class: 10,
            max_centroid_drift: 0.30,
        }
    }
}

/// Cosine similarity between two equal-length vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() { return 0.0; }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na < f32::EPSILON || nb < f32::EPSILON { 0.0 } else { dot / (na * nb) }
}

/// Compute new centroid as a blend of old centroid + mean of gold
/// embeddings. Aborts via error if drift exceeds threshold.
pub fn compute_new_centroid(
    old_centroid: &[f32],
    gold_embeddings: &[Vec<f32>],
    cfg: &CentroidUpdateConfig,
) -> ActiveLearningResult<Vec<f32>> {
    if old_centroid.is_empty() {
        return Err(ActiveLearningError::InvalidInput("old_centroid empty".into()));
    }
    let m = gold_embeddings.len();
    if m < cfg.min_gold_examples_per_class as usize {
        // Insufficient evidence — keep old centroid as-is.
        return Ok(old_centroid.to_vec());
    }

    let d = old_centroid.len();
    for emb in gold_embeddings {
        if emb.len() != d {
            return Err(ActiveLearningError::InvalidInput(format!(
                "embedding dimension mismatch : expected {} got {}", d, emb.len()
            )));
        }
    }

    // Mean of gold embeddings.
    let mut gold_mean = vec![0.0_f32; d];
    let m_f = m as f32;
    for emb in gold_embeddings {
        for (i, v) in emb.iter().enumerate() { gold_mean[i] += v / m_f; }
    }

    // Blend.
    let alpha = cfg.anchor_weight.clamp(0.0, 1.0);
    let new: Vec<f32> = old_centroid.iter().zip(gold_mean.iter())
        .map(|(o, g)| alpha * o + (1.0 - alpha) * g)
        .collect();

    // Safety : drift check.
    let drift = 1.0 - cosine_similarity(old_centroid, &new);
    if drift > cfg.max_centroid_drift {
        return Err(ActiveLearningError::CentroidDriftExceeded {
            drift, max: cfg.max_centroid_drift,
        });
    }
    Ok(new)
}

/// Recompute all centroids in a taxonomy snapshot, given gold examples
/// grouped by class.
pub fn recompute_all_centroids(
    old_centroids: &std::collections::HashMap<String, Vec<f32>>,
    gold_per_class: &std::collections::HashMap<String, Vec<Vec<f32>>>,
    cfg: &CentroidUpdateConfig,
) -> ActiveLearningResult<std::collections::HashMap<String, Vec<f32>>> {
    let mut out = std::collections::HashMap::new();
    for (class_id, old_c) in old_centroids {
        let empty = vec![];
        let gold = gold_per_class.get(class_id).unwrap_or(&empty);
        let new_c = compute_new_centroid(old_c, gold, cfg)?;
        out.insert(class_id.clone(), new_c);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> CentroidUpdateConfig {
        CentroidUpdateConfig {
            anchor_weight: 0.70, min_gold_examples_per_class: 2,
            max_centroid_drift: 0.50,
        }
    }

    #[test]
    fn alpha_1_returns_old() {
        let old = vec![1.0_f32, 0.0, 0.0];
        let gold = vec![vec![0.0_f32, 1.0, 0.0]; 5];
        let cfg = CentroidUpdateConfig {
            anchor_weight: 1.0, min_gold_examples_per_class: 2, max_centroid_drift: 1.0,
        };
        let r = compute_new_centroid(&old, &gold, &cfg).unwrap();
        for (a, b) in r.iter().zip(old.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn alpha_0_returns_gold_mean() {
        let old = vec![1.0_f32, 0.0, 0.0];
        let gold = vec![vec![0.0_f32, 1.0, 0.0]; 5];
        let cfg = CentroidUpdateConfig {
            anchor_weight: 0.0, min_gold_examples_per_class: 2,
            max_centroid_drift: 1.0,
        };
        let r = compute_new_centroid(&old, &gold, &cfg).unwrap();
        assert!((r[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn alpha_07_blends() {
        let old = vec![1.0_f32, 0.0];
        let gold = vec![vec![0.0_f32, 1.0]; 3];
        let r = compute_new_centroid(&old, &gold, &cfg()).unwrap();
        // expected : 0.7 * (1, 0) + 0.3 * (0, 1) = (0.7, 0.3)
        assert!((r[0] - 0.7).abs() < 1e-6);
        assert!((r[1] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn below_min_gold_returns_old() {
        let old = vec![1.0_f32, 0.0];
        let gold = vec![vec![0.0_f32, 1.0]];  // only 1 gold, min is 2
        let r = compute_new_centroid(&old, &gold, &cfg()).unwrap();
        assert_eq!(r, old);
    }

    #[test]
    fn drift_exceeded_aborts() {
        let old = vec![1.0_f32, 0.0];
        let gold = vec![vec![-1.0_f32, 0.0]; 5];  // opposite direction
        let cfg = CentroidUpdateConfig {
            anchor_weight: 0.10,  // mostly gold = big drift
            min_gold_examples_per_class: 2,
            max_centroid_drift: 0.30,
        };
        let r = compute_new_centroid(&old, &gold, &cfg);
        assert!(matches!(r, Err(ActiveLearningError::CentroidDriftExceeded { .. })));
    }

    #[test]
    fn empty_old_errors() {
        // `[vec![1.0]; 5]` needs `Vec<f32>: Copy` (false) ; use the
        // `vec!` macro variant which is Clone-based.
        let r = compute_new_centroid(&[], &vec![vec![1.0]; 5], &cfg());
        assert!(r.is_err());
    }

    #[test]
    fn dimension_mismatch_errors() {
        let old = vec![1.0_f32, 0.0];
        let gold = vec![vec![1.0_f32, 0.0, 0.0]; 5];  // 3D vs 2D old
        let r = compute_new_centroid(&old, &gold, &cfg());
        assert!(r.is_err());
    }
}
