//! Template router: maps a [`TaskDag`] to a [`TemplateDistribution`].

use std::collections::BTreeMap;

use crate::errors::{EfeError, Result};
use crate::genesis::cardinality::Cardinality;
use crate::genesis::context::GenesisContext;
use crate::genesis::dag::{TaskClass, TaskDag};
use crate::types::TemplateCode;

/// Tolerance for the weight-simplex invariant.
const SIMPLEX_EPSILON: f32 = 1e-5;

/// Relative weights of templates in the population.
///
/// Invariants:
/// - non-empty
/// - every weight is finite and >= 0
/// - sum equals 1.0 ± 1e-5
#[derive(Clone, Debug)]
pub struct TemplateDistribution {
    weights: Vec<(TemplateCode, f32)>,
}

impl TemplateDistribution {
    /// Construct, validating the invariants.
    ///
    /// Duplicate template codes are merged (weights summed).
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] on empty input, non-finite or
    ///   negative weights, or invalid simplex.
    pub fn new(weights: Vec<(TemplateCode, f32)>) -> Result<Self> {
        if weights.is_empty() {
            return Err(EfeError::InvalidConfig(
                "TemplateDistribution: must have at least one entry".into(),
            ));
        }
        // Merge duplicates via BTreeMap (sorted by TemplateCode).
        let mut merged: BTreeMap<TemplateCode, f32> = BTreeMap::new();
        for (code, w) in weights {
            if !w.is_finite() {
                return Err(EfeError::InvalidConfig(format!(
                    "TemplateDistribution: non-finite weight for {code:?}"
                )));
            }
            if w < 0.0 {
                return Err(EfeError::InvalidConfig(format!(
                    "TemplateDistribution: negative weight {w} for {code:?}"
                )));
            }
            *merged.entry(code).or_insert(0.0) += w;
        }
        let sum: f32 = merged.values().sum();
        if (sum - 1.0).abs() > SIMPLEX_EPSILON {
            return Err(EfeError::InvalidConfig(format!(
                "TemplateDistribution: weights sum to {sum}, expected 1.0 ± {SIMPLEX_EPSILON}"
            )));
        }
        Ok(Self {
            weights: merged.into_iter().collect(),
        })
    }

    /// Iterate over (template, weight) pairs.
    pub fn iter(&self) -> impl Iterator<Item = &(TemplateCode, f32)> {
        self.weights.iter()
    }

    /// Number of templates in the mix.
    #[must_use]
    pub fn len(&self) -> usize {
        self.weights.len()
    }

    /// `true` if no template has non-zero weight.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.weights.is_empty()
    }

    /// Weight for `code`, `0.0` if absent.
    #[must_use]
    pub fn weight(&self, code: TemplateCode) -> f32 {
        self.weights
            .iter()
            .find_map(|(c, w)| if *c == code { Some(*w) } else { None })
            .unwrap_or(0.0)
    }

    /// Distribute `total` agents proportionally using the
    /// largest-remainder method (guarantees `sum == total`).
    ///
    /// Returns a [`Cardinality`] with the per-template counts.
    #[must_use]
    pub fn distribute(&self, total: u32) -> Cardinality {
        if total == 0 || self.weights.is_empty() {
            return Cardinality {
                total: 0,
                per_template: self.weights.iter().map(|(c, _)| (*c, 0u32)).collect(),
            };
        }
        let t_f = f64::from(total);
        // Step 1: floor of ideal allocations + remainders.
        let mut allocations: Vec<(TemplateCode, u32, f64)> = self
            .weights
            .iter()
            .map(|(code, weight)| {
                let ideal = t_f * f64::from(*weight);
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let floor = ideal.floor() as u32;
                let remainder = ideal - ideal.floor();
                (*code, floor, remainder)
            })
            .collect();

        let floor_sum: u32 = allocations.iter().map(|(_, f, _)| *f).sum();
        let mut to_distribute = total.saturating_sub(floor_sum);

        // Step 2: sort by remainder descending and give +1 to the top.
        allocations.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        for alloc in &mut allocations {
            if to_distribute == 0 {
                break;
            }
            alloc.1 += 1;
            to_distribute -= 1;
        }
        // Restore declaration order (by TemplateCode) for determinism.
        allocations.sort_by_key(|(c, _, _)| *c);

        Cardinality {
            total,
            per_template: allocations.into_iter().map(|(c, n, _)| (c, n)).collect(),
        }
    }
}

/// Maps a [`TaskDag`] to a [`TemplateDistribution`].
pub trait TemplateRouter: Send + Sync {
    /// Select the weighted mix of templates to instantiate.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if the router cannot produce a
    ///   valid simplex.
    fn select(&self, dag: &TaskDag, ctx: &GenesisContext) -> Result<TemplateDistribution>;
}

/// Router with a fixed per-class table, blended by the [`TaskDag::task_mix`].
pub struct StaticRouter {
    per_class: [TemplateDistribution; TaskClass::COUNT],
}

impl StaticRouter {
    /// Construct with an explicit per-class table.
    #[must_use]
    pub fn new(per_class: [TemplateDistribution; TaskClass::COUNT]) -> Self {
        Self { per_class }
    }

    /// Default MVP table derived from empirical calibration.
    ///
    /// # Errors
    ///
    /// Unreachable: hard-coded tables satisfy the simplex invariant
    /// by construction. Returned as `Result` to match the builder
    /// style used elsewhere.
    pub fn mvp_defaults() -> Result<Self> {
        use TemplateCode::{NaCrit, NaLog, NaSem};
        Ok(Self::new([
            // FactoidQa: semantic-dominant, light critique.
            TemplateDistribution::new(vec![(NaSem, 0.70), (NaLog, 0.20), (NaCrit, 0.10)])?,
            // MathWord: logic-heavy.
            TemplateDistribution::new(vec![(NaSem, 0.30), (NaLog, 0.60), (NaCrit, 0.10)])?,
            // LogicalInference: dominated by logic.
            TemplateDistribution::new(vec![(NaSem, 0.20), (NaLog, 0.70), (NaCrit, 0.10)])?,
            // Classification: semantic-dominant.
            TemplateDistribution::new(vec![(NaSem, 0.80), (NaLog, 0.10), (NaCrit, 0.10)])?,
            // Summarization: semantic + critic for diversity.
            TemplateDistribution::new(vec![(NaSem, 0.60), (NaLog, 0.20), (NaCrit, 0.20)])?,
        ]))
    }
}

impl TemplateRouter for StaticRouter {
    fn select(&self, dag: &TaskDag, _ctx: &GenesisContext) -> Result<TemplateDistribution> {
        let mut blended: BTreeMap<TemplateCode, f32> = BTreeMap::new();
        for class in TaskClass::MVP_SET {
            let class_prob = dag.task_mix.prob(*class);
            if class_prob <= 0.0 {
                continue;
            }
            for (code, w) in self.per_class[class.index()].iter() {
                *blended.entry(*code).or_insert(0.0) += class_prob * w;
            }
        }
        // Re-normalize defensively against fp drift.
        let sum: f32 = blended.values().sum();
        if sum <= 0.0 {
            return Err(EfeError::InvalidConfig(
                "StaticRouter: blended distribution has zero mass".into(),
            ));
        }
        let normalized: Vec<(TemplateCode, f32)> =
            blended.into_iter().map(|(c, w)| (c, w / sum)).collect();
        TemplateDistribution::new(normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genesis::dag::{TaskDag, TaskMix};
    use crate::types::SwarmId;
    use std::sync::Arc;

    fn mk_ctx() -> GenesisContext {
        GenesisContext::minimal(SwarmId::new())
    }

    fn mk_dag(mix: TaskMix) -> TaskDag {
        TaskDag::from_mix(mix, Arc::from(vec![0.0f32; 8]))
    }

    #[test]
    fn distribution_rejects_empty() {
        assert!(TemplateDistribution::new(vec![]).is_err());
    }

    #[test]
    fn distribution_rejects_negative_weight() {
        let err = TemplateDistribution::new(vec![
            (TemplateCode::NaSem, -0.1),
            (TemplateCode::NaLog, 1.1),
        ])
        .unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn distribution_rejects_nan() {
        let err = TemplateDistribution::new(vec![
            (TemplateCode::NaSem, f32::NAN),
            (TemplateCode::NaLog, 1.0),
        ])
        .unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn distribution_merges_duplicates() {
        let d = TemplateDistribution::new(vec![
            (TemplateCode::NaSem, 0.3),
            (TemplateCode::NaSem, 0.4),
            (TemplateCode::NaLog, 0.3),
        ])
        .unwrap();
        assert_eq!(d.len(), 2);
        assert!((d.weight(TemplateCode::NaSem) - 0.7).abs() < 1e-5);
    }

    #[test]
    fn distribution_distribute_preserves_total() {
        let d = TemplateDistribution::new(vec![
            (TemplateCode::NaSem, 0.5),
            (TemplateCode::NaLog, 0.3),
            (TemplateCode::NaCrit, 0.2),
        ])
        .unwrap();
        let c = d.distribute(100);
        let sum: u32 = c.per_template.iter().map(|(_, n)| *n).sum();
        assert_eq!(sum, 100);
    }

    #[test]
    fn distribution_distribute_tiny_total() {
        let d = TemplateDistribution::new(vec![
            (TemplateCode::NaSem, 0.334),
            (TemplateCode::NaLog, 0.333),
            (TemplateCode::NaCrit, 0.333),
        ])
        .unwrap();
        let c = d.distribute(10);
        let sum: u32 = c.per_template.iter().map(|(_, n)| *n).sum();
        assert_eq!(sum, 10);
    }

    #[test]
    fn distribution_distribute_zero() {
        let d = TemplateDistribution::new(vec![(TemplateCode::NaSem, 1.0)]).unwrap();
        let c = d.distribute(0);
        assert_eq!(c.total, 0);
        assert!(c.per_template.iter().all(|(_, n)| *n == 0));
    }

    #[test]
    fn static_router_mvp_defaults_build() {
        StaticRouter::mvp_defaults().unwrap();
    }

    #[test]
    fn static_router_blends_uniform() {
        let router = StaticRouter::mvp_defaults().unwrap();
        let dag = mk_dag(TaskMix::uniform());
        let dist = router.select(&dag, &mk_ctx()).unwrap();
        assert!(!dist.is_empty());
        let sum: f32 = dist.iter().map(|(_, w)| *w).sum();
        assert!((sum - 1.0).abs() < 1e-5);
    }

    #[test]
    fn static_router_one_hot_matches_class_distribution() {
        let router = StaticRouter::mvp_defaults().unwrap();
        // LogicalInference should give NaLog ≈ 0.70.
        let dag = mk_dag(TaskMix::one_hot(TaskClass::LogicalInference));
        let dist = router.select(&dag, &mk_ctx()).unwrap();
        assert!(dist.weight(TemplateCode::NaLog) > 0.6);
    }

    #[test]
    fn static_router_factoid_is_semantic_heavy() {
        let router = StaticRouter::mvp_defaults().unwrap();
        let dag = mk_dag(TaskMix::one_hot(TaskClass::FactoidQa));
        let dist = router.select(&dag, &mk_ctx()).unwrap();
        assert!(dist.weight(TemplateCode::NaSem) > dist.weight(TemplateCode::NaLog));
        assert!(dist.weight(TemplateCode::NaSem) > dist.weight(TemplateCode::NaCrit));
    }
}
