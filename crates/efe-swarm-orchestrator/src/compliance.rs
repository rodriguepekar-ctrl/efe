// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : per-agent compliance opt-in.
//!
//! Architecture G13 + ADR-10.  Default is `Compliance::None` (no
//! sig verify, no gold gate) ; callers opt in per-agent for
//! audit-sensitive flows.

use async_trait::async_trait;
use efe_nano_sdk::AgentId;
use serde::{Deserialize, Serialize};

/// **N21 + G13** : compliance level for a single AgentNode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Compliance {
    /// **Default** : no compliance checks ; lowest-latency hot path.
    None,
    /// **N18 cross-cutting** : verify publisher Ed25519 sig before
    /// dispatch.  Carries the publisher_id + key_id required to
    /// look up the public key.
    N18Signed {
        publisher_id: efe_nano_sdk::PublisherId,
        key_id: efe_nano_sdk::KeyId,
    },
    /// **N19 cross-cutting** : confirm the agent passed a recent
    /// gold gate run before dispatch.  Carries the GoldSetId.
    N19GoldGated {
        gold_set_id: efe_nano_sdk::GoldSetId,
    },
}

impl Compliance {
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::N18Signed { .. } => "n18_signed",
            Self::N19GoldGated { .. } => "n19_gold_gated",
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

/// **N21** : runs the compliance check for an agent before dispatch.
/// Sprint 13 ships an `InMemoryComplianceRunner` that's
/// permissive ; Sprint 14+ wire the actual N18 SignatureVerifier +
/// N19 GoldSetRegistry.
#[async_trait]
pub trait ComplianceRunner: Send + Sync + 'static {
    async fn check(
        &self,
        agent_id: &AgentId,
        compliance: &Compliance,
    ) -> Result<(), ComplianceError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComplianceError {
    #[error("N18 signature invalid for agent {agent_id}")]
    SignatureInvalid { agent_id: AgentId },

    #[error("N19 gold set missing for agent {agent_id}")]
    GoldSetMissing { agent_id: AgentId },

    #[error("backend : {message}")]
    Backend { message: String },
}

/// **N21** : permissive in-memory runner that always returns Ok.
/// Used in Sprint 13 tests + when the deployer doesn't wire real
/// N18/N19 checks.  Sprint 14 ships a `WiredComplianceRunner` that
/// calls `efe_publisher_identity::SignatureVerifier` and
/// `efe_gold_test_gate::GoldSetRegistry`.
#[derive(Debug, Default)]
pub struct PermissiveComplianceRunner;

#[async_trait]
impl ComplianceRunner for PermissiveComplianceRunner {
    async fn check(
        &self,
        _agent_id: &AgentId,
        _compliance: &Compliance,
    ) -> Result<(), ComplianceError> {
        Ok(())
    }
}

/// **N21** : strict runner that fails-closed on any non-None compliance.
/// Used in tests that simulate "compliance is required but not wired".
#[derive(Debug, Default)]
pub struct StrictUnwiredComplianceRunner;

#[async_trait]
impl ComplianceRunner for StrictUnwiredComplianceRunner {
    async fn check(
        &self,
        agent_id: &AgentId,
        compliance: &Compliance,
    ) -> Result<(), ComplianceError> {
        match compliance {
            Compliance::None => Ok(()),
            Compliance::N18Signed { .. } => Err(ComplianceError::SignatureInvalid {
                agent_id: agent_id.clone(),
            }),
            Compliance::N19GoldGated { .. } => Err(ComplianceError::GoldSetMissing {
                agent_id: agent_id.clone(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{GoldSetId, KeyId, PublisherId};

    fn aid(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    #[test]
    fn compliance_labels() {
        assert_eq!(Compliance::None.label(), "none");
        assert_eq!(
            Compliance::N18Signed {
                publisher_id: PublisherId::new(),
                key_id: KeyId::new(),
            }
            .label(),
            "n18_signed"
        );
        assert_eq!(
            Compliance::N19GoldGated {
                gold_set_id: GoldSetId::new(),
            }
            .label(),
            "n19_gold_gated"
        );
    }

    #[test]
    fn compliance_is_none() {
        assert!(Compliance::None.is_none());
        assert!(!Compliance::N18Signed {
            publisher_id: PublisherId::new(),
            key_id: KeyId::new(),
        }
        .is_none());
    }

    #[test]
    fn compliance_serde_round_trip() {
        let cases = vec![
            Compliance::None,
            Compliance::N18Signed {
                publisher_id: PublisherId::new(),
                key_id: KeyId::new(),
            },
            Compliance::N19GoldGated {
                gold_set_id: GoldSetId::new(),
            },
        ];
        for c in cases {
            let json = serde_json::to_string(&c).unwrap();
            let back: Compliance = serde_json::from_str(&json).unwrap();
            assert_eq!(c, back);
        }
    }

    #[tokio::test]
    async fn permissive_runner_always_ok() {
        let r = PermissiveComplianceRunner;
        r.check(&aid("test"), &Compliance::None).await.unwrap();
        r.check(
            &aid("test"),
            &Compliance::N18Signed {
                publisher_id: PublisherId::new(),
                key_id: KeyId::new(),
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn strict_runner_ok_only_for_none() {
        let r = StrictUnwiredComplianceRunner;
        r.check(&aid("test"), &Compliance::None).await.unwrap();
        let err = r
            .check(
                &aid("test"),
                &Compliance::N18Signed {
                    publisher_id: PublisherId::new(),
                    key_id: KeyId::new(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ComplianceError::SignatureInvalid { .. }));
        let err2 = r
            .check(
                &aid("test"),
                &Compliance::N19GoldGated {
                    gold_set_id: GoldSetId::new(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err2, ComplianceError::GoldSetMissing { .. }));
    }
}
