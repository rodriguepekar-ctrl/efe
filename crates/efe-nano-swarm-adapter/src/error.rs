// SPDX-License-Identifier: UNLICENSED
//! **Sprint 15 — N23** : closed error taxonomy for the adapter.
//!
//! Every adapter dispatch terminates with one of :
//! - `AgentOutcome::Completed { .. }` on success ;
//! - `AgentOutcome::Errored { kind, message }` on a failure that
//!   the adapter can attribute (signature, gold, cache, dispatch
//!   error from the wrapped runner, blackboard write fault) ;
//! - `SwarmAgentError` for a hard transport failure (rare — only
//!   on infrastructure faults).
//!
//! [`AdapterError`] is the internal taxonomy ; it converts both to
//! `SwarmAgentError` (rare hard fault path) and to a tuple
//! `(AgentErrorKind, String)` used to build `AgentOutcome::
//! Errored` for soft faults.

use efe_swarm_orchestrator::{AgentErrorKind, SwarmAgentError};
use thiserror::Error;

/// **N23** : closed taxonomy of adapter failure causes.
///
/// Variants map to either a "soft" outcome (`AgentOutcome::Errored`
/// with a chosen `AgentErrorKind`) or a "hard" `SwarmAgentError`
/// depending on whether the swarm orchestrator should keep the
/// DAG progressing past the failing node.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AdapterError {
    /// The configured `input_pointer` is unset on the blackboard.
    #[error("blackboard input pointer {0:?} is unset")]
    BlackboardReadMissing(String),

    /// The blackboard write failed (size cap, invalid pointer, ...).
    #[error("blackboard write failed at {pointer:?} : {message}")]
    BlackboardWriteFailed {
        /// The JSON pointer that was being written.
        pointer: String,
        /// Underlying error message.
        message: String,
    },

    /// Policy demanded signature verification but the catalogue
    /// entry is unsigned or the verifier rejected the signature.
    #[error("signature gate refused dispatch : {0}")]
    SignatureInvalid(String),

    /// Policy demanded a gold-binding check but the catalogue
    /// entry has no `gold_set_ref`, or the lookup returned a
    /// non-`Approved` status.
    #[error("gold binding gate refused dispatch : {0}")]
    GoldBindingMissing(String),

    /// The idempotency cache returned a transport-level error.
    #[error("idempotency cache failed : {0}")]
    CacheError(String),

    /// The wrapped runner returned an error.
    #[error("dispatch failed : {0}")]
    DispatchFailed(String),

    /// The wrapped runner reported it should be skipped (a
    /// conditional precondition was not met).
    #[error("runner skipped : {0}")]
    Skipped(String),
}

impl AdapterError {
    /// **N23** : the canonical `AgentErrorKind` for this adapter
    /// error.  Used when surfacing as
    /// `AgentOutcome::Errored { kind, message }`.
    pub fn agent_error_kind(&self) -> AgentErrorKind {
        match self {
            Self::BlackboardReadMissing(_) | Self::BlackboardWriteFailed { .. } => {
                AgentErrorKind::BlackboardError
            }
            Self::SignatureInvalid(_) | Self::GoldBindingMissing(_) => {
                AgentErrorKind::ComplianceFailed
            }
            Self::CacheError(_) => AgentErrorKind::Backend,
            Self::DispatchFailed(_) => AgentErrorKind::ProviderError,
            Self::Skipped(_) => AgentErrorKind::Backend,
        }
    }

    /// **N23** : true if this error is "soft" — the swarm should
    /// keep the DAG progressing past the failing node by reporting
    /// `AgentOutcome::Errored { .. }` rather than aborting.
    pub fn is_soft(&self) -> bool {
        match self {
            Self::SignatureInvalid(_)
            | Self::GoldBindingMissing(_)
            | Self::CacheError(_)
            | Self::DispatchFailed(_)
            | Self::Skipped(_)
            | Self::BlackboardReadMissing(_) => true,
            // Blackboard writes are transport-level ; the swarm
            // can't safely keep going if we couldn't even record
            // the output.
            Self::BlackboardWriteFailed { .. } => false,
        }
    }
}

impl From<AdapterError> for SwarmAgentError {
    fn from(e: AdapterError) -> Self {
        match e {
            AdapterError::BlackboardWriteFailed { message, .. }
            | AdapterError::BlackboardReadMissing(message)
                if message.is_empty() =>
            {
                SwarmAgentError::Blackboard {
                    message: "blackboard fault".into(),
                }
            }
            AdapterError::BlackboardWriteFailed { pointer, message } => {
                SwarmAgentError::Blackboard {
                    message: format!("write {pointer} : {message}"),
                }
            }
            AdapterError::BlackboardReadMissing(p) => SwarmAgentError::Blackboard {
                message: format!("missing pointer {p}"),
            },
            other => SwarmAgentError::Backend {
                message: other.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_failure_maps_to_compliance() {
        let e = AdapterError::SignatureInvalid("bad sig".into());
        assert_eq!(e.agent_error_kind(), AgentErrorKind::ComplianceFailed);
        assert!(e.is_soft());
    }

    #[test]
    fn gold_failure_maps_to_compliance() {
        let e = AdapterError::GoldBindingMissing("no set".into());
        assert_eq!(e.agent_error_kind(), AgentErrorKind::ComplianceFailed);
        assert!(e.is_soft());
    }

    #[test]
    fn dispatch_failure_maps_to_provider() {
        let e = AdapterError::DispatchFailed("upstream timeout".into());
        assert_eq!(e.agent_error_kind(), AgentErrorKind::ProviderError);
    }

    #[test]
    fn blackboard_write_is_hard_failure() {
        let e = AdapterError::BlackboardWriteFailed {
            pointer: "/x".into(),
            message: "cap exceeded".into(),
        };
        assert_eq!(e.agent_error_kind(), AgentErrorKind::BlackboardError);
        assert!(!e.is_soft());
    }

    #[test]
    fn blackboard_read_missing_is_soft() {
        let e = AdapterError::BlackboardReadMissing("/input/text".into());
        assert!(e.is_soft());
    }

    #[test]
    fn into_swarm_agent_error_blackboard_path() {
        let e = AdapterError::BlackboardWriteFailed {
            pointer: "/x".into(),
            message: "cap exceeded".into(),
        };
        let se: SwarmAgentError = e.into();
        assert!(matches!(se, SwarmAgentError::Blackboard { .. }));
    }

    #[test]
    fn into_swarm_agent_error_backend_path() {
        let e = AdapterError::DispatchFailed("boom".into());
        let se: SwarmAgentError = e.into();
        assert!(matches!(se, SwarmAgentError::Backend { .. }));
    }

    #[test]
    fn display_messages_include_context() {
        let e = AdapterError::SignatureInvalid("forged".into());
        assert!(e.to_string().contains("forged"));
        let e = AdapterError::BlackboardWriteFailed {
            pointer: "/x".into(),
            message: "cap".into(),
        };
        assert!(e.to_string().contains("/x"));
        assert!(e.to_string().contains("cap"));
    }
}
