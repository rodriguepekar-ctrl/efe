//! Cardinality: total agent count and per-template split.

use crate::errors::{EfeError, Result};
use crate::genesis::context::GenesisContext;
use crate::genesis::dag::TaskDag;
use crate::genesis::router::TemplateDistribution;
use crate::types::{PopulationMix, TemplateCode};

/// Concrete population plan: total count and per-template breakdown.
///
/// Invariants:
/// - `total` equals the sum of `per_template[_].1`.
/// - `per_template` is non-empty.
#[derive(Clone, Debug)]
pub struct Cardinality {
    /// Total number of nano-agents to instantiate.
    pub total: u32,
    /// Per-template counts. Sum equals `total`.
    pub per_template: Vec<(TemplateCode, u32)>,
}

impl Cardinality {
    /// Verify `total` matches the sum of per-template counts.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] when the cardinality is malformed.
    pub fn validate(&self) -> Result<()> {
        if self.per_template.is_empty() {
            return Err(EfeError::InvalidConfig(
                "Cardinality: per_template must be non-empty".into(),
            ));
        }
        // Detect duplicate template codes — the consumer (spawner) relies
        // on uniqueness when building the registry-local prior index.
        let mut seen = std::collections::BTreeSet::new();
        for (code, _) in &self.per_template {
            if !seen.insert(*code) {
                return Err(EfeError::InvalidConfig(format!(
                    "Cardinality: duplicate template {code:?}"
                )));
            }
        }
        let sum: u32 = self.per_template.iter().map(|(_, n)| *n).sum();
        if sum != self.total {
            return Err(EfeError::InvalidConfig(format!(
                "Cardinality: per-template sum {sum} != total {}",
                self.total
            )));
        }
        Ok(())
    }

    /// Convert to M1's [`PopulationMix`] for `SwarmSpec::population`.
    ///
    /// Drops entries with zero count; [`PopulationMix`] requires a
    /// positive total (already guaranteed by the cardinality estimator
    /// lower bound).
    ///
    /// # Errors
    ///
    /// - Propagates errors from [`PopulationMix::new`].
    pub fn as_population_mix(&self) -> Result<PopulationMix> {
        let filtered: Vec<(TemplateCode, u32)> = self
            .per_template
            .iter()
            .copied()
            .filter(|(_, n)| *n > 0)
            .collect();
        PopulationMix::new(filtered)
    }
}

// =====================================================================
// Sprint **M1 extension D33** — Balanced GA helpers.
// =====================================================================

/// **Sprint D33** — Construct a [`PopulationMix`] using the GA template
/// set's `default_share()` values.
///
/// Pre-D33 callers had to manually compute per-template counts ; this
/// helper does the math from the canonical share values declared in
/// `crates/efe-core/src/agents/templates/`.
///
/// **Invariant** : the sum of `default_share()` across all GA
/// templates is `1.0` (verified by
/// `crate::agents::templates::tests::ga_default_shares_sum_to_unity`).
/// This function multiplies each share by `total_pop` and rounds to
/// the nearest integer ; entries with `count == 0` are filtered (they
/// would violate `PopulationMix::new`'s positive-count invariant).
///
/// `total_pop` MUST be ≥ 8 to give each GA template at least 1 agent.
/// Below 8, the function returns a [`PopulationMix`] containing only
/// the templates whose share rounded to ≥ 1.
///
/// # Errors
///
/// Propagates errors from [`PopulationMix::new`] (e.g. all-zero
/// counts after rounding).
///
/// # Examples
///
/// ```no_run
/// use efe_core::genesis::cardinality::balanced_ga;
/// let mix = balanced_ga(80).expect("sum 1.0 × 80 = 80 agents");
/// // mix has 8 entries with counts proportional to default_share.
/// ```
pub fn balanced_ga(total_pop: u32) -> Result<PopulationMix> {
    use crate::agents::templates::ga_templates;

    let templates = ga_templates();
    let mut per: Vec<(TemplateCode, u32)> = templates
        .iter()
        .map(|t| {
            let share = t.default_share();
            let count = (share * total_pop as f32).round() as u32;
            (t.code(), count)
        })
        .filter(|(_, n)| *n > 0)
        .collect();

    // **Sprint D33** — rounding can produce a sum slightly above or
    // below `total_pop`. We adjust the largest entry (NaSem at 45 %
    // share) to absorb the rounding delta. This keeps the sum
    // exactly equal to `total_pop` so downstream consumers
    // (Cardinality::validate) don't trip on a sum mismatch.
    let sum: u32 = per.iter().map(|(_, n)| *n).sum();
    if sum != total_pop && !per.is_empty() {
        // Find the index of the entry with the highest count.
        let (idx, _) = per
            .iter()
            .enumerate()
            .max_by_key(|(_, (_, n))| *n)
            .expect("per non-empty by construction");
        if sum < total_pop {
            per[idx].1 += total_pop - sum;
        } else {
            // sum > total_pop ; the largest entry can absorb the
            // overshoot since its count > delta in practice.
            let delta = sum - total_pop;
            per[idx].1 = per[idx].1.saturating_sub(delta);
        }
    }

    PopulationMix::new(per)
}

/// **Sprint D33** — MVP-only balanced helper.
///
/// Backward-compat counterpart to [`balanced_ga`] — uses only the
/// 3 MVP templates. Existing catalogs that opt into `template_set:
/// "mvp"` go through this path.
pub fn balanced_mvp(total_pop: u32) -> Result<PopulationMix> {
    use crate::agents::templates::mvp_templates;

    let templates = mvp_templates();
    let mut per: Vec<(TemplateCode, u32)> = templates
        .iter()
        .map(|t| {
            // MVP shares as defined by the trait : NaSem 0.45,
            // NaLog 0.05, NaCrit 0.05. The MVP-only mix renormalises
            // these to sum 1.0 across the 3 entries.
            let share = t.default_share();
            (t.code(), share)
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|(c, s)| (c, s / 0.55 /* = 0.45 + 0.05 + 0.05 */))
        .map(|(c, s)| (c, (s * total_pop as f32).round() as u32))
        .filter(|(_, n)| *n > 0)
        .collect();

    // Adjust for rounding (same pattern as balanced_ga).
    let sum: u32 = per.iter().map(|(_, n)| *n).sum();
    if sum != total_pop && !per.is_empty() {
        let (idx, _) = per
            .iter()
            .enumerate()
            .max_by_key(|(_, (_, n))| *n)
            .expect("per non-empty by construction");
        if sum < total_pop {
            per[idx].1 += total_pop - sum;
        } else {
            let delta = sum - total_pop;
            per[idx].1 = per[idx].1.saturating_sub(delta);
        }
    }

    PopulationMix::new(per)
}

/// Computes a [`Cardinality`] from a [`TaskDag`] + [`TemplateDistribution`].
pub trait CardinalityEstimator: Send + Sync {
    /// Estimate the concrete population plan.
    ///
    /// # Errors
    ///
    /// Returns an error if inputs violate estimator-specific constraints.
    fn estimate(
        &self,
        dag: &TaskDag,
        distribution: &TemplateDistribution,
        ctx: &GenesisContext,
    ) -> Result<Cardinality>;
}

/// Heuristic estimator using
/// `N = clamp(min, max, base * sqrt(entropy) * ln(1/ε))`.
#[derive(Clone, Debug)]
pub struct HeuristicEstimator {
    /// Hard lower bound on agent count.
    pub min_agents: u32,
    /// Hard upper bound on agent count.
    pub max_agents: u32,
    /// Multiplicative factor (calibrated empirically).
    pub base_factor: f32,
    /// Target convergence error used in `ln(1/ε)`.
    pub epsilon_target: f32,
}

impl Default for HeuristicEstimator {
    fn default() -> Self {
        Self {
            min_agents: 128,
            max_agents: 1024,
            base_factor: 128.0,
            epsilon_target: 1e-3,
        }
    }
}

impl HeuristicEstimator {
    /// Validate configuration.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] on inconsistent bounds.
    pub fn validate(&self) -> Result<()> {
        if self.min_agents == 0 {
            return Err(EfeError::InvalidConfig(
                "HeuristicEstimator: min_agents must be >= 1".into(),
            ));
        }
        if self.max_agents < self.min_agents {
            return Err(EfeError::InvalidConfig(format!(
                "HeuristicEstimator: max_agents ({}) < min_agents ({})",
                self.max_agents, self.min_agents
            )));
        }
        if !self.base_factor.is_finite() || self.base_factor <= 0.0 {
            return Err(EfeError::InvalidConfig(
                "HeuristicEstimator: base_factor must be positive and finite".into(),
            ));
        }
        if !self.epsilon_target.is_finite() || self.epsilon_target <= 0.0 {
            return Err(EfeError::InvalidConfig(
                "HeuristicEstimator: epsilon_target must be positive and finite".into(),
            ));
        }
        Ok(())
    }
}

impl CardinalityEstimator for HeuristicEstimator {
    fn estimate(
        &self,
        dag: &TaskDag,
        distribution: &TemplateDistribution,
        _ctx: &GenesisContext,
    ) -> Result<Cardinality> {
        self.validate()?;
        // Clamp entropy to a sane range to keep the scale factor tame.
        let max_h = (dag.task_mix.as_slice().len() as f32).ln(); // ln(N_classes)
        let h = dag.entropy_estimate.clamp(0.1, max_h.max(0.1));
        let log_eps = (1.0 / self.epsilon_target).ln();
        let raw = self.base_factor * h.sqrt() * log_eps;
        // Guard against non-finite or negative raw (shouldn't happen, defensive).
        let raw_u = if raw.is_finite() && raw >= 0.0 {
            raw.round() as u32
        } else {
            self.min_agents
        };
        let total = raw_u.clamp(self.min_agents, self.max_agents);
        Ok(distribution.distribute(total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genesis::dag::{TaskClass, TaskMix};
    use crate::types::SwarmId;
    use std::sync::Arc;

    fn mk_ctx() -> GenesisContext {
        GenesisContext::minimal(SwarmId::new())
    }

    fn mk_dag(entropy: f32) -> TaskDag {
        // Fabricate a task mix that yields approximately the requested
        // entropy. We use interpolation between one-hot and uniform.
        let uniform = TaskMix::uniform();
        let one_hot = TaskMix::one_hot(TaskClass::FactoidQa);
        let max_h = (TaskClass::COUNT as f32).ln();
        let alpha = (entropy / max_h).clamp(0.0, 1.0);
        let probs: Vec<f32> = uniform
            .as_slice()
            .iter()
            .zip(one_hot.as_slice())
            .map(|(u, o)| alpha * u + (1.0 - alpha) * o)
            .collect();
        let mix = TaskMix::new(probs).unwrap();
        TaskDag::from_mix(mix, Arc::from(vec![0.0f32; 4]))
    }

    fn mk_distribution() -> TemplateDistribution {
        TemplateDistribution::new(vec![
            (TemplateCode::NaSem, 0.5),
            (TemplateCode::NaLog, 0.3),
            (TemplateCode::NaCrit, 0.2),
        ])
        .unwrap()
    }

    #[test]
    fn cardinality_validate_rejects_sum_mismatch() {
        let c = Cardinality {
            total: 100,
            per_template: vec![(TemplateCode::NaSem, 60), (TemplateCode::NaLog, 30)],
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn cardinality_validate_accepts_valid() {
        let c = Cardinality {
            total: 10,
            per_template: vec![(TemplateCode::NaSem, 7), (TemplateCode::NaLog, 3)],
        };
        c.validate().unwrap();
    }

    #[test]
    fn cardinality_as_population_mix_drops_zeros() {
        let c = Cardinality {
            total: 7,
            per_template: vec![
                (TemplateCode::NaSem, 7),
                (TemplateCode::NaLog, 0),
                (TemplateCode::NaCrit, 0),
            ],
        };
        let mix = c.as_population_mix().unwrap();
        assert_eq!(mix.template_count(), 1);
        assert_eq!(mix.total(), 7);
    }

    #[test]
    fn heuristic_clamps_to_min() {
        let est = HeuristicEstimator {
            min_agents: 200,
            max_agents: 300,
            base_factor: 0.001, // tiny → would produce very small total
            epsilon_target: 1e-3,
        };
        let dag = mk_dag(0.1);
        let dist = mk_distribution();
        let c = est.estimate(&dag, &dist, &mk_ctx()).unwrap();
        assert!(c.total >= 200);
        c.validate().unwrap();
    }

    #[test]
    fn heuristic_clamps_to_max() {
        let est = HeuristicEstimator {
            min_agents: 64,
            max_agents: 256,
            base_factor: 10_000.0, // huge → would produce large total
            epsilon_target: 1e-3,
        };
        let dag = mk_dag(1.2);
        let dist = mk_distribution();
        let c = est.estimate(&dag, &dist, &mk_ctx()).unwrap();
        assert_eq!(c.total, 256);
    }

    #[test]
    fn heuristic_monotonic_in_entropy() {
        let est = HeuristicEstimator::default();
        let dist = mk_distribution();
        let c_low = est.estimate(&mk_dag(0.1), &dist, &mk_ctx()).unwrap();
        let c_high = est.estimate(&mk_dag(1.5), &dist, &mk_ctx()).unwrap();
        assert!(c_high.total >= c_low.total);
    }

    #[test]
    fn heuristic_rejects_bad_config() {
        let est = HeuristicEstimator {
            min_agents: 0,
            ..Default::default()
        };
        let dag = mk_dag(0.5);
        let dist = mk_distribution();
        assert!(est.estimate(&dag, &dist, &mk_ctx()).is_err());
    }

    #[test]
    fn heuristic_result_passes_validate() {
        let est = HeuristicEstimator::default();
        let dag = mk_dag(0.7);
        let dist = mk_distribution();
        let c = est.estimate(&dag, &dist, &mk_ctx()).unwrap();
        c.validate().unwrap();
    }
}
