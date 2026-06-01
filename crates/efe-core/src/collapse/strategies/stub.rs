//! `StubStrategy` — test-only strategy that records its invocations.
//!
//! Used by unit/integration tests to assert the `CollapseEngine`'s
//! escalation logic without running actual recovery math.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use crate::collapse::context::CollapseContext;
use crate::collapse::outcome::{CollapseAction, CollapseOutcome};
use crate::collapse::strategy::CollapseStrategy;
use crate::collapse::trigger::CollapseTrigger;
use crate::errors::Result;

/// Scripted outcome behavior for a stub strategy.
#[derive(Clone, Debug)]
pub enum StubBehavior {
    /// Always return `success = true`.
    AlwaysSucceed,
    /// Always return `success = false` but recoverable (no fatal warning).
    AlwaysFail,
    /// Declare this trigger out-of-scope (return `applicable_to = false`).
    NotApplicable,
    /// Fail for the first `at_call - 1` invocations, succeed from the `at_call`-th onward.
    SucceedAt {
        /// 1-based invocation index at which success begins.
        at_call: u32,
    },
}

/// Stub strategy for tests.
///
/// Exposes a public `call_count` counter so tests can assert how many
/// times the engine invoked this particular strategy.
#[derive(Debug)]
pub struct StubStrategy {
    name: &'static str,
    behavior: StubBehavior,
    pub(crate) call_count: Arc<AtomicU32>,
}

impl StubStrategy {
    /// Create a new stub with the given name and behavior.
    #[must_use]
    pub fn new(name: &'static str, behavior: StubBehavior) -> Self {
        Self {
            name,
            behavior,
            call_count: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Return a handle to the counter. Useful for tests that want to
    /// check the count from outside.
    #[must_use]
    pub fn counter(&self) -> Arc<AtomicU32> {
        self.call_count.clone()
    }

    /// Current call count.
    #[must_use]
    pub fn calls(&self) -> u32 {
        self.call_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl CollapseStrategy for StubStrategy {
    async fn apply(&self, ctx: &mut CollapseContext<'_>) -> Result<CollapseOutcome> {
        let start = Instant::now();
        let n = self.call_count.fetch_add(1, Ordering::SeqCst) + 1;

        let success = match &self.behavior {
            StubBehavior::AlwaysSucceed => true,
            StubBehavior::AlwaysFail | StubBehavior::NotApplicable => false,
            StubBehavior::SucceedAt { at_call } => n >= *at_call,
        };

        let action = if success {
            CollapseAction::SoftRollback { to_round: 0 }
        } else {
            CollapseAction::NoOp
        };

        Ok(CollapseOutcome {
            action_taken: action,
            success,
            trigger: ctx.trigger.clone(),
            new_round: if success { Some(ctx.round) } else { None },
            variance_after: Some(0.0),
            recovery_confidence: if success { 1.0 } else { 0.0 },
            warnings: vec![],
            duration_us: u32::try_from(start.elapsed().as_micros()).unwrap_or(u32::MAX),
            strategy_name: self.name.into(),
        })
    }

    fn applicable_to(&self, _trigger: &CollapseTrigger) -> bool {
        !matches!(self.behavior, StubBehavior::NotApplicable)
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_counter_increments() {
        let s = StubStrategy::new("test", StubBehavior::AlwaysSucceed);
        assert_eq!(s.calls(), 0);
        let c = s.counter();
        c.fetch_add(1, Ordering::SeqCst);
        assert_eq!(s.calls(), 1);
    }

    #[test]
    fn not_applicable_returns_false() {
        let s = StubStrategy::new("x", StubBehavior::NotApplicable);
        let t = CollapseTrigger::Manual {
            reason: "x".into(),
            round: 0,
        };
        assert!(!s.applicable_to(&t));
    }

    #[test]
    fn applicable_otherwise() {
        let s = StubStrategy::new("x", StubBehavior::AlwaysSucceed);
        let t = CollapseTrigger::Manual {
            reason: "x".into(),
            round: 0,
        };
        assert!(s.applicable_to(&t));
    }
}
