//! `CollapseEvent` — telemetry record of one collapse call.

use serde::{Deserialize, Serialize};

use crate::collapse::outcome::CollapseOutcome;
use crate::collapse::trigger::CollapseTrigger;
use crate::types::SwarmId;

/// Record of one collapse operation, consumable by M11 for telemetry.
///
/// # Timestamp caveats
///
/// [`timestamp_nanos`] is a **wall-clock** reading
/// (`SystemTime::now().duration_since(UNIX_EPOCH)`). It can go backward
/// across events if the host clock is adjusted (NTP sync, leap second,
/// VM suspend/resume). Consumers that need a strictly monotonic
/// ordering should sort by the tuple `(collapse_index, timestamp_nanos)`
/// for a given swarm.
///
/// [`timestamp_nanos`]: Self::timestamp_nanos
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CollapseEvent {
    /// Unix nanoseconds since epoch at the moment the collapse started.
    /// May be non-monotonic under wall-clock jumps — see struct docs.
    pub timestamp_nanos: u64,
    /// Which swarm was collapsed.
    pub swarm_id: SwarmId,
    /// What triggered the collapse.
    pub trigger: CollapseTrigger,
    /// Full outcome of the collapse.
    pub outcome: CollapseOutcome,
    /// 1-based index of this collapse for the swarm (i.e. if this is
    /// the 3rd collapse for this swarm, `collapse_index = 3`).
    pub collapse_index: u32,
}

impl CollapseEvent {
    /// Convenience: short label for metric cardinality.
    #[must_use]
    pub fn label(&self) -> (&'static str, &'static str) {
        (self.trigger.label(), self.outcome.action_taken.label())
    }
}

impl std::fmt::Display for CollapseEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "event[{idx}]@{ts}ns swarm={swarm:?} {outcome}",
            idx = self.collapse_index,
            ts = self.timestamp_nanos,
            swarm = self.swarm_id,
            outcome = self.outcome,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collapse::outcome::CollapseAction;
    use crate::consensus::verdict::AnomalyCode;

    fn mk_event() -> CollapseEvent {
        let trigger = CollapseTrigger::FromAnomaly {
            code: AnomalyCode::NanInf,
            round: 3,
        };
        let outcome = CollapseOutcome {
            action_taken: CollapseAction::SoftRollback { to_round: 2 },
            success: true,
            trigger: trigger.clone(),
            new_round: Some(2),
            variance_after: Some(0.05),
            recovery_confidence: 0.9,
            warnings: vec![],
            duration_us: 800,
            strategy_name: "soft-rollback".into(),
        };
        CollapseEvent {
            timestamp_nanos: 1_234_567_890,
            swarm_id: SwarmId::new(),
            trigger,
            outcome,
            collapse_index: 1,
        }
    }

    #[test]
    fn label_tuple_is_stable() {
        let e = mk_event();
        let (trig, act) = e.label();
        assert_eq!(trig, "anomaly");
        assert_eq!(act, "soft_rollback");
    }

    #[test]
    fn serde_round_trip() {
        let e = mk_event();
        let s = serde_json::to_string(&e).unwrap();
        let back: CollapseEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);
    }
}
