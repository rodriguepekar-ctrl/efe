//! `FullAbortStrategy` — terminal level-5 recovery.
//!
//! Always returns `FullAbort`. Does not modify the ELS. The engine
//! places it last in the escalation ladder as the default fallthrough.

use std::time::Instant;

use async_trait::async_trait;

use crate::collapse::context::CollapseContext;
use crate::collapse::outcome::{CollapseAction, CollapseOutcome, CollapseWarning};
use crate::collapse::strategy::CollapseStrategy;
use crate::collapse::trigger::CollapseTrigger;
use crate::errors::Result;

/// Terminal strategy: declares the swarm unrecoverable.
#[derive(Debug, Default, Clone)]
pub struct FullAbortStrategy;

#[async_trait]
impl CollapseStrategy for FullAbortStrategy {
    async fn apply(&self, ctx: &mut CollapseContext<'_>) -> Result<CollapseOutcome> {
        let start = Instant::now();
        Ok(CollapseOutcome {
            action_taken: CollapseAction::FullAbort,
            success: false,
            trigger: ctx.trigger.clone(),
            new_round: None,
            variance_after: None,
            recovery_confidence: 0.0,
            warnings: vec![CollapseWarning::Abandoned],
            duration_us: u32::try_from(start.elapsed().as_micros()).unwrap_or(u32::MAX),
            strategy_name: self.name().into(),
        })
    }

    fn applicable_to(&self, _trigger: &CollapseTrigger) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "full-abort"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::verdict::AnomalyCode;

    #[test]
    fn always_applicable() {
        let s = FullAbortStrategy;
        let triggers = [
            CollapseTrigger::FromAnomaly {
                code: AnomalyCode::NanInf,
                round: 0,
            },
            CollapseTrigger::FromStall {
                rounds_flat: 0,
                confidence: 0.0,
                round: 0,
            },
            CollapseTrigger::Manual {
                reason: String::new(),
                round: 0,
            },
        ];
        for t in triggers {
            assert!(s.applicable_to(&t));
        }
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(FullAbortStrategy.name(), "full-abort");
    }
}
