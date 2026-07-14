//! **Couche 3 · M1 Palier-2 — fixed (x-independent) genesis topology.**
//!
//! The genesis pipeline's dynamic stages make the swarm TOPOLOGY a function
//! of the input embedding `x` : `meta.decompose(x)` classifies `x`, the
//! router blends per-class template distributions by that classification,
//! and the cardinality estimator maps its entropy to an **integer** agent
//! count.  `N(x)` and the template mix are therefore **step functions of
//! `x`** — so the one-round participant operator `Φ` is *discontinuous at
//! cell boundaries*, where its local Lipschitz ratio is unbounded.  That
//! discontinuity is the one mechanism that can break the meta-swarm's H1
//! hypothesis (`L_Φ ≤ 1`) for an otherwise non-expansive pipeline
//! (architecture doc `COUCHE3-M1-vrais-participants-ARCHITECTURE.md` §1.2).
//!
//! This module provides the Palier-2 remedy : a router and a cardinality
//! estimator whose outputs are **constants** fixed at construction.  Wired
//! into a [`crate::genesis::GenesisEngine`] (via
//! `builder().router(..).cardinality(..)`), they make the spawned population
//! — count, template mix, and therefore the active mask — **independent of
//! `x`** : the only remaining `x`-dependence in a fusion round is the
//! continuous `beta_mix` stage, so `Φ` becomes **continuous by
//! construction** (no routing cells, no boundaries to jump).
//!
//! ## Honesty (never over-sell)
//!
//! Continuity-by-construction removes the *discontinuity* threat to H1 ;
//! it does **not** by itself certify a contraction modulus.  The
//! architecture doc's closed-form claim (`L_Φ ≤ β₁ ≈ 0.17` intra-cell) is
//! **not** minted as a certificate anywhere : the M1.c empirical canary
//! measured `L̂ ≈ 0.69` on the real operator, and until that gap is
//! reconciled the only honest H1 evidence remains the **measured** bound
//! (property-tested, NOT certified).  Fixed topology + canary is the
//! honest pairing the doc itself prescribes (§3, « le mélange »).

use crate::errors::{EfeError, Result};
use crate::genesis::cardinality::{Cardinality, CardinalityEstimator};
use crate::genesis::context::GenesisContext;
use crate::genesis::dag::TaskDag;
use crate::genesis::router::{TemplateDistribution, TemplateRouter};
use crate::types::TemplateCode;

/// A router that returns the SAME template distribution for every input —
/// the `dag` (i.e. everything derived from `x`) is deliberately ignored.
#[derive(Clone, Debug)]
pub struct FixedTemplateRouter {
    distribution: TemplateDistribution,
}

impl FixedTemplateRouter {
    /// Build from an explicit `(template, count)` population plan — the same
    /// plan handed to [`FixedCardinality`], so the router's weights and the
    /// cardinality's counts are consistent by construction.
    ///
    /// # Errors
    ///
    /// [`EfeError::InvalidConfig`] on an empty plan, a duplicate template
    /// code, or a zero count (all fail-closed : a malformed plan must never
    /// silently degrade to some default topology).
    pub fn from_population(plan: &[(TemplateCode, u32)]) -> Result<Self> {
        let (total, _) = validate_plan(plan)?;
        let weights: Vec<(TemplateCode, f32)> = plan
            .iter()
            .map(|(code, n)| (*code, *n as f32 / total as f32))
            .collect();
        Ok(Self {
            distribution: TemplateDistribution::new(weights)?,
        })
    }
}

impl TemplateRouter for FixedTemplateRouter {
    fn select(&self, _dag: &TaskDag, _ctx: &GenesisContext) -> Result<TemplateDistribution> {
        // `_dag` carries everything x-dependent — ignored BY DESIGN.
        Ok(self.distribution.clone())
    }
}

/// A cardinality estimator that returns the SAME population plan for every
/// input — count and per-template mix are constants fixed at construction.
#[derive(Clone, Debug)]
pub struct FixedCardinality {
    plan: Cardinality,
}

impl FixedCardinality {
    /// Build from an explicit `(template, count)` population plan.
    ///
    /// # Errors
    ///
    /// [`EfeError::InvalidConfig`] on an empty plan, duplicate template
    /// codes, or zero counts ;  plus anything
    /// [`Cardinality::validate`] rejects (defence in depth).
    pub fn from_population(plan: &[(TemplateCode, u32)]) -> Result<Self> {
        let (total, per_template) = validate_plan(plan)?;
        let plan = Cardinality {
            total,
            per_template,
        };
        plan.validate()?;
        Ok(Self { plan })
    }

    /// The constant total agent count.
    #[must_use]
    pub fn total(&self) -> u32 {
        self.plan.total
    }
}

impl CardinalityEstimator for FixedCardinality {
    fn estimate(
        &self,
        _dag: &TaskDag,
        _distribution: &TemplateDistribution,
        _ctx: &GenesisContext,
    ) -> Result<Cardinality> {
        // Both x-dependent inputs ignored BY DESIGN : the topology is a
        // constant of the participant's construction, not of the query.
        Ok(self.plan.clone())
    }
}

/// Shared fail-closed validation of a population plan.
fn validate_plan(plan: &[(TemplateCode, u32)]) -> Result<(u32, Vec<(TemplateCode, u32)>)> {
    if plan.is_empty() {
        return Err(EfeError::InvalidConfig(
            "fixed topology: the population plan must not be empty".into(),
        ));
    }
    let mut seen: Vec<TemplateCode> = Vec::with_capacity(plan.len());
    let mut total: u32 = 0;
    for (code, n) in plan {
        if seen.contains(code) {
            return Err(EfeError::InvalidConfig(format!(
                "fixed topology: duplicate template code {code:?} in the population plan",
            )));
        }
        seen.push(*code);
        if *n == 0 {
            return Err(EfeError::InvalidConfig(format!(
                "fixed topology: template {code:?} has count 0",
            )));
        }
        total = total.checked_add(*n).ok_or_else(|| {
            EfeError::InvalidConfig("fixed topology: population total overflows u32".into())
        })?;
    }
    Ok((total, plan.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genesis::dag::{TaskClass, TaskMix};
    use crate::types::SwarmId;

    fn plan() -> Vec<(TemplateCode, u32)> {
        vec![(TemplateCode::NaSem, 4), (TemplateCode::NaLog, 2)]
    }

    /// Two maximally-different dags (uniform mix vs one-hot, wildly different
    /// entropy) MUST produce bit-identical routing and cardinality — the
    /// whole point of Palier-2 : no trace of `x` reaches the topology.
    #[test]
    fn outputs_are_independent_of_the_dag() {
        let router = FixedTemplateRouter::from_population(&plan()).unwrap();
        let card = FixedCardinality::from_population(&plan()).unwrap();
        let ctx_a = GenesisContext::minimal(SwarmId::new());
        let ctx_b = GenesisContext::minimal(SwarmId::new());

        // Two maximally-different x-derived views : uniform mix over a
        // positive embedding vs a one-hot mix over a negative one.
        let dag_a = TaskDag {
            task_mix: TaskMix::uniform(),
            entropy_estimate: 2.0,
            primary_class: TaskClass::FactoidQa,
            problem_embedding: vec![0.3_f32; 8].into(),
        };
        let dag_b = TaskDag {
            task_mix: TaskMix::one_hot(TaskClass::LogicalInference),
            entropy_estimate: 0.0,
            primary_class: TaskClass::LogicalInference,
            problem_embedding: vec![-0.9_f32; 8].into(),
        };

        let dist_a = router.select(&dag_a, &ctx_a).unwrap();
        let dist_b = router.select(&dag_b, &ctx_b).unwrap();
        assert_eq!(format!("{dist_a:?}"), format!("{dist_b:?}"), "routing must ignore the dag");

        let card_a = card.estimate(&dag_a, &dist_a, &ctx_a).unwrap();
        let card_b = card.estimate(&dag_b, &dist_b, &ctx_b).unwrap();
        assert_eq!(card_a.total, 6);
        assert_eq!(card_a.total, card_b.total);
        assert_eq!(card_a.per_template, card_b.per_template, "cardinality must ignore the dag");
    }

    /// The plan is honoured EXACTLY (counts, not heuristics — no hidden
    /// `min_agents=128` floor like the HeuristicEstimator's).
    #[test]
    fn plan_is_honoured_exactly() {
        let card = FixedCardinality::from_population(&plan()).unwrap();
        let dag = TaskDag {
            task_mix: TaskMix::uniform(),
            entropy_estimate: 1.0,
            primary_class: TaskClass::FactoidQa,
            problem_embedding: vec![0.1_f32; 8].into(),
        };
        let dist = FixedTemplateRouter::from_population(&plan())
            .unwrap()
            .select(&dag, &GenesisContext::minimal(SwarmId::new()))
            .unwrap();
        let c = card
            .estimate(&dag, &dist, &GenesisContext::minimal(SwarmId::new()))
            .unwrap();
        assert_eq!(c.total, 6);
        assert_eq!(
            c.per_template,
            vec![(TemplateCode::NaSem, 4), (TemplateCode::NaLog, 2)],
        );
        c.validate().expect("plan satisfies Cardinality invariants");
    }

    #[test]
    fn empty_plan_fails_closed() {
        assert!(FixedTemplateRouter::from_population(&[]).is_err());
        assert!(FixedCardinality::from_population(&[]).is_err());
    }

    #[test]
    fn duplicate_template_fails_closed() {
        let dup = vec![(TemplateCode::NaSem, 2), (TemplateCode::NaSem, 3)];
        assert!(FixedTemplateRouter::from_population(&dup).is_err());
        assert!(FixedCardinality::from_population(&dup).is_err());
    }

    #[test]
    fn zero_count_fails_closed() {
        let zero = vec![(TemplateCode::NaSem, 0)];
        assert!(FixedCardinality::from_population(&zero).is_err());
    }
}
