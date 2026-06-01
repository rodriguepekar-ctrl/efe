// SPDX-License-Identifier: UNLICENSED
//! M33 — Multi-hop query planner.
//!
//! Translates a natural-language / typed query into a sequence of
//! steps that the executor runs. M33 ships a **deterministic**
//! planner ; M33.1 will add an LLM-based planner.

use serde::{Deserialize, Serialize};

use crate::error::{GraphRagError, Result};

/// Maximum number of steps in a plan.
pub const MAX_PLAN_STEPS: usize = 16;

/// One step in the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanStep {
    /// Dense vector search seeded by the query text/vector.
    DenseSeed {
        /// Number of candidates to retrieve.
        top_k: usize,
    },
    /// Graph BFS from current seeds.
    GraphExpand {
        /// Hop limit.
        max_hops: usize,
        /// Per-hop fan-out cap.
        max_results: usize,
    },
    /// Re-rank everything by fused score and truncate.
    Rank {
        /// Final top_k.
        top_k: usize,
    },
}

/// Compiled query plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryPlan {
    /// Ordered steps.
    pub steps: Vec<PlanStep>,
}

impl QueryPlan {
    /// Validate the plan.
    pub fn validate(&self) -> Result<()> {
        if self.steps.is_empty() {
            return Err(GraphRagError::Internal {
                reason: "empty plan".into(),
            });
        }
        if self.steps.len() > MAX_PLAN_STEPS {
            return Err(GraphRagError::PlanTooLong {
                observed: self.steps.len(),
                cap: MAX_PLAN_STEPS,
            });
        }
        // Plan must end with a Rank step.
        if !matches!(self.steps.last(), Some(PlanStep::Rank { .. })) {
            return Err(GraphRagError::Internal {
                reason: "plan must end with Rank".into(),
            });
        }
        Ok(())
    }
}

/// Construct a default 3-step plan : DenseSeed → GraphExpand → Rank.
///
/// `top_k` is the user-requested final result count ; the dense seed
/// retrieves `2 × top_k` candidates and the graph expansion fans
/// out to `4 × top_k`.
pub fn plan_query(top_k: usize, max_hops: usize) -> Result<QueryPlan> {
    if top_k == 0 {
        return Err(GraphRagError::Internal {
            reason: "top_k must be > 0".into(),
        });
    }
    let dense_k = top_k.saturating_mul(2).min(1000);
    let graph_k = top_k.saturating_mul(4).min(2000);
    let plan = QueryPlan {
        steps: vec![
            PlanStep::DenseSeed { top_k: dense_k },
            PlanStep::GraphExpand {
                max_hops,
                max_results: graph_k,
            },
            PlanStep::Rank { top_k },
        ],
    };
    plan.validate()?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_query_basic() {
        let p = plan_query(10, 2).unwrap();
        assert_eq!(p.steps.len(), 3);
        assert!(matches!(p.steps[0], PlanStep::DenseSeed { top_k: 20 }));
        assert!(matches!(
            p.steps[1],
            PlanStep::GraphExpand {
                max_hops: 2,
                max_results: 40
            }
        ));
        assert!(matches!(p.steps[2], PlanStep::Rank { top_k: 10 }));
    }

    #[test]
    fn plan_query_zero_top_k_rejected() {
        let e = plan_query(0, 2).unwrap_err();
        assert!(matches!(e, GraphRagError::Internal { .. }));
    }

    #[test]
    fn plan_validate_rejects_empty() {
        let p = QueryPlan { steps: vec![] };
        assert!(p.validate().is_err());
    }

    #[test]
    fn plan_validate_rejects_too_long() {
        let p = QueryPlan {
            steps: vec![PlanStep::DenseSeed { top_k: 1 }; MAX_PLAN_STEPS + 1],
        };
        assert!(matches!(
            p.validate(),
            Err(GraphRagError::PlanTooLong { .. })
        ));
    }

    #[test]
    fn plan_validate_requires_terminal_rank() {
        let p = QueryPlan {
            steps: vec![PlanStep::DenseSeed { top_k: 10 }],
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn plan_caps_at_2000_graph_results() {
        let p = plan_query(usize::MAX, 2).unwrap();
        if let PlanStep::GraphExpand { max_results, .. } = p.steps[1] {
            assert_eq!(max_results, 2000);
        } else {
            panic!("expected GraphExpand step");
        }
    }
}
