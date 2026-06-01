// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : SwarmAgent trait + AgentNode + AgentOutcome.

use std::sync::Arc;

use async_trait::async_trait;
use efe_blackboard::{Blackboard, EventBus};
use efe_nano_sdk::{AgentId, SwarmId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::compliance::Compliance;

/// **N21** : context handed to a SwarmAgent at dispatch time.
#[derive(Clone)]
pub struct SwarmAgentContext {
    pub swarm_id: SwarmId,
    pub agent_id: AgentId,
    pub blackboard: Arc<Blackboard>,
    pub eventbus: Arc<EventBus>,
    /// The original swarm input (raw user payload).
    pub input: Arc<serde_json::Value>,
    /// Deterministic per-agent seed derived from `(swarm_id, agent_id)`.
    /// Replay determinism (I13) — agents that need RNG should use
    /// this seed, not `OsRng`.
    pub rng_seed: u64,
}

impl std::fmt::Debug for SwarmAgentContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwarmAgentContext")
            .field("swarm_id", &self.swarm_id)
            .field("agent_id", &self.agent_id)
            .field("rng_seed", &self.rng_seed)
            .finish_non_exhaustive()
    }
}

/// **N21** : the agent trait.
#[async_trait]
pub trait SwarmAgent: Send + Sync + 'static {
    fn agent_id(&self) -> AgentId;
    async fn run(&self, ctx: SwarmAgentContext) -> Result<AgentOutcome, SwarmAgentError>;
}

/// **N21** : the outcome of a single agent run.  Includes Completed
/// (success), Errored (recoverable failure), TimedOut, and Skipped
/// (conditional edge predicate said no).
///
/// Serde tag is `outcome` (not `kind`) because the `Errored` variant
/// carries a `kind: AgentErrorKind` field — serde rejects tag
/// collision with a struct field (lesson from N19 audit B4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AgentOutcome {
    Completed {
        cost_microcents: i64,
        duration_ms: u32,
        blackboard_writes: u32,
        events_published: u32,
    },
    Errored {
        kind: AgentErrorKind,
        message: String,
    },
    TimedOut {
        timeout_ms: u32,
    },
    Skipped {
        reason: String,
    },
}

impl AgentOutcome {
    /// **N21** : Stable label for metrics.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Completed { .. } => "completed",
            Self::Errored { .. } => "errored",
            Self::TimedOut { .. } => "timed_out",
            Self::Skipped { .. } => "skipped",
        }
    }

    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Completed { .. })
    }

    pub fn is_errored(&self) -> bool {
        matches!(self, Self::Errored { .. })
    }

    pub fn cost_microcents(&self) -> i64 {
        match self {
            Self::Completed {
                cost_microcents, ..
            } => *cost_microcents,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentErrorKind {
    /// Agent panicked.
    Panic,
    /// Agent's underlying provider (LLM, DB) returned an error.
    ProviderError,
    /// Blackboard write conflict / size limit / pointer invalid.
    BlackboardError,
    /// Compliance check failed (N18 sig invalid, N19 gold missing).
    ComplianceFailed,
    /// Other backend / infra fault.
    Backend,
}

impl AgentErrorKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Panic => "panic",
            Self::ProviderError => "provider_error",
            Self::BlackboardError => "blackboard_error",
            Self::ComplianceFailed => "compliance_failed",
            Self::Backend => "backend",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum SwarmAgentError {
    #[error("provider : {message}")]
    Provider { message: String },

    #[error("blackboard : {message}")]
    Blackboard { message: String },

    #[error("backend : {message}")]
    Backend { message: String },
}

impl SwarmAgentError {
    pub fn kind(&self) -> AgentErrorKind {
        match self {
            Self::Provider { .. } => AgentErrorKind::ProviderError,
            Self::Blackboard { .. } => AgentErrorKind::BlackboardError,
            Self::Backend { .. } => AgentErrorKind::Backend,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  AgentNode — DAG node = (agent + timeout + compliance)
// ─────────────────────────────────────────────────────────────────────

pub struct AgentNode {
    pub agent_id: AgentId,
    pub agent: Arc<dyn SwarmAgent>,
    pub timeout_ms: u32,
    pub compliance: Compliance,
}

impl std::fmt::Debug for AgentNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentNode")
            .field("agent_id", &self.agent_id)
            .field("timeout_ms", &self.timeout_ms)
            .field("compliance", &self.compliance.label())
            .finish_non_exhaustive()
    }
}

impl AgentNode {
    /// **N21** : Default per-agent timeout (5 s).
    pub const DEFAULT_TIMEOUT_MS: u32 = 5_000;
    /// **N21** : Maximum per-agent timeout (60 s).
    pub const MAX_TIMEOUT_MS: u32 = 60_000;

    pub fn new(agent: Arc<dyn SwarmAgent>) -> Self {
        let agent_id = agent.agent_id();
        Self {
            agent_id,
            agent,
            timeout_ms: Self::DEFAULT_TIMEOUT_MS,
            compliance: Compliance::None,
        }
    }

    pub fn with_timeout_ms(mut self, ms: u32) -> Self {
        self.timeout_ms = ms.min(Self::MAX_TIMEOUT_MS);
        self
    }

    pub fn with_compliance(mut self, c: Compliance) -> Self {
        self.compliance = c;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aid(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    struct EchoAgent {
        id: AgentId,
    }
    #[async_trait]
    impl SwarmAgent for EchoAgent {
        fn agent_id(&self) -> AgentId {
            self.id.clone()
        }
        async fn run(&self, _ctx: SwarmAgentContext) -> Result<AgentOutcome, SwarmAgentError> {
            Ok(AgentOutcome::Completed {
                cost_microcents: 100,
                duration_ms: 1,
                blackboard_writes: 0,
                events_published: 0,
            })
        }
    }

    #[test]
    fn agent_outcome_labels_stable() {
        let completed = AgentOutcome::Completed {
            cost_microcents: 1,
            duration_ms: 1,
            blackboard_writes: 0,
            events_published: 0,
        };
        assert_eq!(completed.label(), "completed");
        assert!(completed.is_completed());
        assert!(!completed.is_errored());
        assert_eq!(completed.cost_microcents(), 1);

        let errored = AgentOutcome::Errored {
            kind: AgentErrorKind::Panic,
            message: "x".into(),
        };
        assert_eq!(errored.label(), "errored");
        assert!(errored.is_errored());
        assert_eq!(errored.cost_microcents(), 0);

        let timed = AgentOutcome::TimedOut { timeout_ms: 5_000 };
        assert_eq!(timed.label(), "timed_out");

        let skipped = AgentOutcome::Skipped {
            reason: "cond".into(),
        };
        assert_eq!(skipped.label(), "skipped");
    }

    #[test]
    fn agent_error_kind_labels() {
        assert_eq!(AgentErrorKind::Panic.label(), "panic");
        assert_eq!(AgentErrorKind::ProviderError.label(), "provider_error");
        assert_eq!(AgentErrorKind::BlackboardError.label(), "blackboard_error");
        assert_eq!(AgentErrorKind::ComplianceFailed.label(), "compliance_failed");
        assert_eq!(AgentErrorKind::Backend.label(), "backend");
    }

    #[test]
    fn swarm_agent_error_kind_mapping() {
        assert_eq!(
            SwarmAgentError::Provider {
                message: "x".into()
            }
            .kind(),
            AgentErrorKind::ProviderError
        );
        assert_eq!(
            SwarmAgentError::Blackboard {
                message: "x".into()
            }
            .kind(),
            AgentErrorKind::BlackboardError
        );
        assert_eq!(
            SwarmAgentError::Backend {
                message: "x".into()
            }
            .kind(),
            AgentErrorKind::Backend
        );
    }

    #[test]
    fn agent_node_default_timeout() {
        let n = AgentNode::new(Arc::new(EchoAgent { id: aid("echo") }));
        assert_eq!(n.timeout_ms, AgentNode::DEFAULT_TIMEOUT_MS);
        assert!(matches!(n.compliance, Compliance::None));
    }

    #[test]
    fn agent_node_custom_timeout_capped() {
        let n = AgentNode::new(Arc::new(EchoAgent { id: aid("echo") }))
            .with_timeout_ms(120_000);
        assert_eq!(n.timeout_ms, AgentNode::MAX_TIMEOUT_MS);
    }

    #[test]
    fn agent_node_custom_timeout_within_max() {
        let n = AgentNode::new(Arc::new(EchoAgent { id: aid("echo") }))
            .with_timeout_ms(30_000);
        assert_eq!(n.timeout_ms, 30_000);
    }

    #[test]
    fn outcome_serde_round_trip() {
        let cases = vec![
            AgentOutcome::Completed {
                cost_microcents: 1,
                duration_ms: 1,
                blackboard_writes: 0,
                events_published: 0,
            },
            AgentOutcome::Errored {
                kind: AgentErrorKind::ProviderError,
                message: "x".into(),
            },
            AgentOutcome::TimedOut { timeout_ms: 5_000 },
            AgentOutcome::Skipped {
                reason: "cond".into(),
            },
        ];
        for c in cases {
            let json = serde_json::to_string(&c).unwrap();
            let back: AgentOutcome = serde_json::from_str(&json).unwrap();
            assert_eq!(c, back);
        }
    }

    #[test]
    fn agent_node_debug_no_panic() {
        let n = AgentNode::new(Arc::new(EchoAgent { id: aid("echo") }));
        let _ = format!("{n:?}");
    }
}
