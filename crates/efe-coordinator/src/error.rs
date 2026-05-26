// SPDX-License-Identifier: UNLICENSED
//! Taxonomie d'erreurs du Coordinator — 10 variantes typées.
//!
//! Conçue pour englober proprement [`efe_nano_app_coordinator::RobotError`]
//! tout en ajoutant les erreurs propres à la couche d'orchestration :
//! - dépassement de la blackboard,
//! - cycle dans le DAG,
//! - vote sans quorum,
//! - panne d'un LLM gros,
//! - timeout global du workflow,
//! - etc.

use std::time::Duration;

use efe_nano_app_coordinator::RobotError;

/// Résultat shorthand utilisé par toute la crate.
pub type CoordResult<T> = std::result::Result<T, CoordError>;

/// Erreurs canoniques du Coordinator.
#[derive(Debug, thiserror::Error)]
pub enum CoordError {
    /// Un nano-robot a échoué — encapsule l'erreur Phase 1.
    #[error("nano-robot error: {0}")]
    Robot(#[from] RobotError),

    /// Un LLM gros (Claude/GPT/Gemini/...) a échoué.
    #[error("LLM provider '{provider}' failed: {reason}")]
    LlmProvider {
        /// Nom du provider (`claude_opus_4`, `gpt4_turbo`, ...).
        provider: String,
        /// Cause humaine.
        reason: String,
    },

    /// La blackboard est pleine (limite dure 256 Ko).
    #[error("blackboard full: {used_bytes} / {max_bytes} bytes (entry of {entry_bytes} bytes rejected)")]
    BlackboardFull {
        /// Octets actuellement consommés.
        used_bytes: usize,
        /// Borne dure.
        max_bytes: usize,
        /// Taille de l'entrée rejetée.
        entry_bytes: usize,
    },

    /// Le DAG soumis contient un cycle (devrait être impossible si construit
    /// via [`crate::dag::DagExecutor`] — utile pour les tests sandbox).
    #[error("DAG cycle detected involving node '{node_id}'")]
    DagCycle {
        /// ID du nœud où le cycle a été détecté.
        node_id: String,
    },

    /// Le DAG référence un parent inexistant.
    #[error("DAG node '{node_id}' depends on missing parent '{missing_parent}'")]
    DagMissingParent {
        /// Nœud dépendant.
        node_id: String,
        /// Parent absent.
        missing_parent: String,
    },

    /// Vote du `VoteEngine` n'a pas atteint le quorum requis.
    #[error("vote failed to reach quorum: {ballots_received}/{quorum_required} ballots")]
    VoteNoQuorum {
        /// Nombre de bulletins reçus.
        ballots_received: usize,
        /// Quorum requis.
        quorum_required: usize,
    },

    /// Mission B appelée avec moins de 2 LLMs (impossible de voter).
    #[error("mission B requires ≥ 2 LLMs, got {n}")]
    MissionBTooFewLlms {
        /// Nombre fourni.
        n: usize,
    },

    /// Le workflow a dépassé son `max_total_latency`.
    #[error("workflow timeout after {ms}ms (deadline {deadline_ms}ms)")]
    WorkflowTimeout {
        /// Durée écoulée.
        ms: u64,
        /// Deadline imposée.
        deadline_ms: u64,
    },

    /// Subscriber SSE est trop lent — buffer plein.
    #[error("SSE subscriber slow: {dropped} events dropped on token {token:?}")]
    SubscriberLag {
        /// Token du subscriber.
        token: String,
        /// Évènements supprimés.
        dropped: u64,
    },

    /// Erreur de configuration au boot.
    #[error("coordinator config error: {0}")]
    Config(String),
}

impl CoordError {
    /// L'erreur est-elle réessayable au niveau du workflow ?
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Robot(e) => matches!(
                e,
                RobotError::Transport { .. } | RobotError::Timeout { .. } | RobotError::HttpStatus { .. }
            ),
            Self::LlmProvider { .. } => true,
            Self::WorkflowTimeout { .. } => false,
            Self::BlackboardFull { .. } => false,
            Self::DagCycle { .. } => false,
            Self::DagMissingParent { .. } => false,
            Self::VoteNoQuorum { .. } => false,
            Self::MissionBTooFewLlms { .. } => false,
            Self::SubscriberLag { .. } => false,
            Self::Config(_) => false,
        }
    }

    /// Backoff suggéré pour le caller si l'erreur est retryable.
    #[must_use]
    pub fn suggested_backoff(&self) -> Duration {
        match self {
            Self::Robot(e) => e.suggested_backoff(0),
            Self::LlmProvider { .. } => Duration::from_millis(500),
            _ => Duration::from_secs(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_robot_error_is_implemented() {
        let r: CoordError = RobotError::Config {
            robot: "analyzer",
            reason: "test".into(),
        }
        .into();
        assert!(matches!(r, CoordError::Robot(_)));
        assert!(!r.is_retryable());
    }

    #[test]
    fn llm_provider_is_retryable() {
        let e = CoordError::LlmProvider {
            provider: "claude_opus_4".into(),
            reason: "503".into(),
        };
        assert!(e.is_retryable());
        assert_eq!(e.suggested_backoff(), Duration::from_millis(500));
    }

    #[test]
    fn workflow_timeout_not_retryable() {
        let e = CoordError::WorkflowTimeout {
            ms: 5000,
            deadline_ms: 4000,
        };
        assert!(!e.is_retryable());
    }

    #[test]
    fn vote_no_quorum_not_retryable() {
        let e = CoordError::VoteNoQuorum {
            ballots_received: 1,
            quorum_required: 2,
        };
        assert!(!e.is_retryable());
    }

    #[test]
    fn display_messages_are_informative() {
        let e = CoordError::BlackboardFull {
            used_bytes: 250_000,
            max_bytes: 262_144,
            entry_bytes: 20_000,
        };
        let s = format!("{e}");
        assert!(s.contains("262144"), "msg = {s}");
        assert!(s.contains("250000"), "msg = {s}");
    }
}
