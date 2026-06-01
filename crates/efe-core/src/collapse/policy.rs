//! `CollapsePolicy` — ordered list of strategies to try during escalation.

use std::sync::Arc;

use crate::collapse::strategies::{
    CentroidProjectionStrategy, FullAbortStrategy, NoiseInjectionStrategy, PartialRespawnStrategy,
    SoftRollbackStrategy,
};
use crate::collapse::strategy::CollapseStrategy;

/// Ordered list of strategies tried by the engine, and a retry budget
/// per `handle()` call.
#[derive(Clone)]
pub struct CollapsePolicy {
    /// Strategies to attempt in order. The engine stops at the first
    /// strategy whose outcome `is_recoverable()`.
    pub escalation_order: Vec<Arc<dyn CollapseStrategy>>,
    /// Retry budget per `engine.handle()` call. Currently informational;
    /// the engine honors it by scanning each strategy once per call.
    pub max_retries_per_call: u32,
}

impl std::fmt::Debug for CollapsePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&'static str> = self.escalation_order.iter().map(|s| s.name()).collect();
        f.debug_struct("CollapsePolicy")
            .field("escalation_order", &names)
            .field("max_retries_per_call", &self.max_retries_per_call)
            .finish()
    }
}

impl CollapsePolicy {
    /// Construct a custom policy from an ordered list of strategies.
    #[must_use]
    pub fn custom(strategies: Vec<Arc<dyn CollapseStrategy>>) -> Self {
        Self {
            escalation_order: strategies,
            max_retries_per_call: 1,
        }
    }

    /// MVP defaults: `SoftRollback → NoiseInjection → CentroidProjection
    /// → PartialRespawn → FullAbort`.
    #[must_use]
    pub fn default_mvp() -> Self {
        Self {
            escalation_order: vec![
                Arc::new(SoftRollbackStrategy),
                Arc::new(NoiseInjectionStrategy),
                Arc::new(CentroidProjectionStrategy),
                Arc::new(PartialRespawnStrategy),
                Arc::new(FullAbortStrategy),
            ],
            max_retries_per_call: 1,
        }
    }

    /// Number of strategies registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.escalation_order.len()
    }

    /// True iff the policy has no strategies — pathological but valid.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.escalation_order.is_empty()
    }

    /// Iterate over the strategy names in escalation order.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.escalation_order.iter().map(|s| s.name())
    }
}

impl Default for CollapsePolicy {
    fn default() -> Self {
        Self::default_mvp()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collapse::strategies::{StubBehavior, StubStrategy};

    #[test]
    fn default_mvp_has_five_levels() {
        let p = CollapsePolicy::default_mvp();
        assert_eq!(p.len(), 5);
        let names: Vec<&str> = p.names().collect();
        assert_eq!(
            names,
            vec![
                "soft-rollback",
                "noise-injection",
                "centroid-projection",
                "partial-respawn",
                "full-abort",
            ]
        );
    }

    #[test]
    fn custom_policy_respects_order() {
        let s1: Arc<dyn CollapseStrategy> =
            Arc::new(StubStrategy::new("a", StubBehavior::AlwaysFail));
        let s2: Arc<dyn CollapseStrategy> =
            Arc::new(StubStrategy::new("b", StubBehavior::AlwaysSucceed));
        let p = CollapsePolicy::custom(vec![s1, s2]);
        let names: Vec<&str> = p.names().collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn empty_policy_is_empty() {
        let p = CollapsePolicy::custom(vec![]);
        assert!(p.is_empty());
        assert_eq!(p.len(), 0);
    }

    #[test]
    fn debug_format_lists_names() {
        let p = CollapsePolicy::default_mvp();
        let s = format!("{p:?}");
        assert!(s.contains("soft-rollback"));
        assert!(s.contains("full-abort"));
    }
}
