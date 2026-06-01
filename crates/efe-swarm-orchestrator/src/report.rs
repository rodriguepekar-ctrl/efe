// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : SwarmReport — § 7.3.

use std::time::SystemTime;

use efe_nano_sdk::{AgentId, ContextBundle, SwarmId};
use serde::{Deserialize, Serialize};

use crate::agent::AgentOutcome;

/// **N21** : per-agent row in the swarm report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentReport {
    pub agent_id: AgentId,
    pub outcome: AgentOutcome,
    pub started_at: SystemTime,
    pub completed_at: SystemTime,
}

impl AgentReport {
    /// **N21** : wall-clock duration of this agent's execution.
    pub fn duration_ms(&self) -> u64 {
        self.completed_at
            .duration_since(self.started_at)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// **N21** : the full swarm run report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwarmReport {
    pub swarm_id: SwarmId,
    pub started_at: SystemTime,
    pub completed_at: SystemTime,
    pub agents: Vec<AgentReport>,
    pub total_cost_microcents: i64,
    pub bundle: ContextBundle,
    pub overall: SwarmOverallVerdict,
}

impl SwarmReport {
    /// **N21** : wall-clock total duration.
    pub fn duration_ms(&self) -> u64 {
        self.completed_at
            .duration_since(self.started_at)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// **N21** : Number of agents whose outcome was Completed.
    pub fn agents_completed(&self) -> usize {
        self.agents.iter().filter(|a| a.outcome.is_completed()).count()
    }

    /// **N21** : Number of agents whose outcome was Errored.
    pub fn agents_errored(&self) -> usize {
        self.agents.iter().filter(|a| a.outcome.is_errored()).count()
    }
}

/// **N21** : overall swarm verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SwarmOverallVerdict {
    /// All agents Completed (or Skipped via predicate, which is fine).
    Passed { agents_total: u32 },
    /// At least one agent Errored but the aggregator still ran ;
    /// the bundle is partial but usable.
    PartialSuccess {
        agents_completed: u32,
        agents_errored: u32,
        agents_total: u32,
    },
    /// The aggregator itself failed.  Bundle is empty / missing.
    Failed { reason: String },
    /// Global swarm timeout fired.
    Aborted { reason: String },
}

impl SwarmOverallVerdict {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Passed { .. } => "passed",
            Self::PartialSuccess { .. } => "partial_success",
            Self::Failed { .. } => "failed",
            Self::Aborted { .. } => "aborted",
        }
    }

    pub fn is_passed(&self) -> bool {
        matches!(self, Self::Passed { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentErrorKind;
    use efe_nano_sdk::AgentId;

    fn aid(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    fn completed_report(name: &str, cost: i64) -> AgentReport {
        AgentReport {
            agent_id: aid(name),
            outcome: AgentOutcome::Completed {
                cost_microcents: cost,
                duration_ms: 5,
                blackboard_writes: 1,
                events_published: 0,
            },
            started_at: SystemTime::UNIX_EPOCH,
            completed_at: SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(5),
        }
    }

    fn errored_report(name: &str) -> AgentReport {
        AgentReport {
            agent_id: aid(name),
            outcome: AgentOutcome::Errored {
                kind: AgentErrorKind::Panic,
                message: "x".into(),
            },
            started_at: SystemTime::UNIX_EPOCH,
            completed_at: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn agent_report_duration_ms() {
        let r = completed_report("ner_extractor", 100);
        assert_eq!(r.duration_ms(), 5);
    }

    #[test]
    fn swarm_report_completed_errored_counts() {
        let r = SwarmReport {
            swarm_id: SwarmId::new(),
            started_at: SystemTime::UNIX_EPOCH,
            completed_at: SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(10),
            agents: vec![
                completed_report("aaa", 1),
                completed_report("bbb", 2),
                errored_report("ccc"),
            ],
            total_cost_microcents: 3,
            bundle: ContextBundle::empty(SwarmId::new()),
            overall: SwarmOverallVerdict::PartialSuccess {
                agents_completed: 2,
                agents_errored: 1,
                agents_total: 3,
            },
        };
        assert_eq!(r.agents_completed(), 2);
        assert_eq!(r.agents_errored(), 1);
        assert_eq!(r.duration_ms(), 10);
    }

    #[test]
    fn verdict_labels_stable() {
        assert_eq!(SwarmOverallVerdict::Passed { agents_total: 1 }.label(), "passed");
        assert_eq!(
            SwarmOverallVerdict::PartialSuccess {
                agents_completed: 1,
                agents_errored: 1,
                agents_total: 2
            }
            .label(),
            "partial_success"
        );
        assert_eq!(
            SwarmOverallVerdict::Failed {
                reason: "x".into()
            }
            .label(),
            "failed"
        );
        assert_eq!(
            SwarmOverallVerdict::Aborted {
                reason: "x".into()
            }
            .label(),
            "aborted"
        );
    }

    #[test]
    fn verdict_is_passed() {
        assert!(SwarmOverallVerdict::Passed { agents_total: 1 }.is_passed());
        assert!(!SwarmOverallVerdict::Failed {
            reason: "x".into()
        }
        .is_passed());
    }

    #[test]
    fn report_serde_round_trip() {
        let r = SwarmReport {
            swarm_id: SwarmId::new(),
            started_at: SystemTime::UNIX_EPOCH,
            completed_at: SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(10),
            agents: vec![completed_report("aaa", 1)],
            total_cost_microcents: 1,
            bundle: ContextBundle::empty(SwarmId::new()),
            overall: SwarmOverallVerdict::Passed { agents_total: 1 },
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: SwarmReport = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
