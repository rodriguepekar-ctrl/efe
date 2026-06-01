//! Strongly-typed topic constants. Use these instead of string literals
//! to catch typos at compile time.
//!
//! New topics added later just add a const + variant.

/// All known M28 topics. The string form is what flows over NATS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    Classify(ClassifyTopic),
    Agent(AgentTopic),
    Drift(DriftTopic),
    Bundle(BundleTopic),
    Batch(BatchTopic),
    Dpia(DpiaTopic),
    Graduation(GraduationTopic),
    Audit(AuditTopic),
}

impl Topic {
    /// Wire representation, e.g. `classify.completed`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Topic::Classify(t) => t.as_str(),
            Topic::Agent(t) => t.as_str(),
            Topic::Drift(t) => t.as_str(),
            Topic::Bundle(t) => t.as_str(),
            Topic::Batch(t) => t.as_str(),
            Topic::Dpia(t) => t.as_str(),
            Topic::Graduation(t) => t.as_str(),
            Topic::Audit(t) => t.as_str(),
        }
    }
}

impl std::fmt::Display for Topic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClassifyTopic {
    Completed,
    Failed,
}

impl ClassifyTopic {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "classify.completed",
            Self::Failed => "classify.failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentTopic {
    RunCreated,
    RunCompleted,
    RunFailed,
    RunTerminated,
    ApprovalRequested,
    ApprovalGranted,
    ApprovalRejected,
}

impl AgentTopic {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RunCreated => "agent.run.created",
            Self::RunCompleted => "agent.run.completed",
            Self::RunFailed => "agent.run.failed",
            Self::RunTerminated => "agent.run.terminated",
            Self::ApprovalRequested => "agent.approval.requested",
            Self::ApprovalGranted => "agent.approval.granted",
            Self::ApprovalRejected => "agent.approval.rejected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DriftTopic {
    Detected,
}

impl DriftTopic {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Detected => "drift.detected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BundleTopic {
    Deployed,
    RolledBack,
}

impl BundleTopic {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Deployed => "bundle.deployed",
            Self::RolledBack => "bundle.rolled_back",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BatchTopic {
    Submitted,
    Completed,
    Failed,
    Cancelled,
}

impl BatchTopic {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Submitted => "batch.submitted",
            Self::Completed => "batch.completed",
            Self::Failed => "batch.failed",
            Self::Cancelled => "batch.cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DpiaTopic {
    Signed,
    Expired,
}

impl DpiaTopic {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Signed => "dpia.signed",
            Self::Expired => "dpia.expired",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GraduationTopic {
    Acknowledged,
    Revoked,
}

impl GraduationTopic {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Acknowledged => "graduation.acknowledged",
            Self::Revoked => "graduation.revoked",
        }
    }
}

/// System-level audit events (cross-tenant ; published on
/// `efe.system.audit.*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuditTopic {
    CachePoisoned,
    JwtRevocation,
    QuotaExceeded,
    AdminAction,
}

impl AuditTopic {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CachePoisoned => "audit.cache_poisoned",
            Self::JwtRevocation => "audit.jwt_revocation",
            Self::QuotaExceeded => "audit.quota_exceeded",
            Self::AdminAction => "audit.admin_action",
        }
    }
}

/// The set of topics customers can subscribe to in `POST /v1/webhooks`.
/// Wildcards not supported in v1 (must list explicitly).
pub const PUBLIC_SUBSCRIBABLE_TOPICS: &[&str] = &[
    "classify.completed",
    "classify.failed",
    "agent.run.created",
    "agent.run.completed",
    "agent.run.failed",
    "agent.run.terminated",
    "agent.approval.requested",
    "agent.approval.granted",
    "agent.approval.rejected",
    "drift.detected",
    "bundle.deployed",
    "bundle.rolled_back",
    "batch.submitted",
    "batch.completed",
    "batch.failed",
    "dpia.signed",
    "dpia.expired",
    "graduation.acknowledged",
    "graduation.revoked",
];

/// Returns true if `topic` is in the public-subscribable list.
pub fn is_public_subscribable(topic: &str) -> bool {
    PUBLIC_SUBSCRIBABLE_TOPICS.contains(&topic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_topic_strings_are_stable() {
        assert_eq!(ClassifyTopic::Completed.as_str(), "classify.completed");
        assert_eq!(ClassifyTopic::Failed.as_str(), "classify.failed");
    }

    #[test]
    fn topic_display_matches_as_str() {
        let t = Topic::Classify(ClassifyTopic::Completed);
        assert_eq!(t.to_string(), "classify.completed");
    }

    #[test]
    fn public_subscribable_includes_all_user_topics() {
        assert!(is_public_subscribable("classify.completed"));
        assert!(is_public_subscribable("agent.approval.requested"));
        assert!(!is_public_subscribable("audit.cache_poisoned"));   // system-only
        assert!(!is_public_subscribable("nonexistent.topic"));
    }

    #[test]
    fn no_duplicate_topic_strings() {
        let mut seen = std::collections::HashSet::new();
        for topic in PUBLIC_SUBSCRIBABLE_TOPICS {
            assert!(seen.insert(*topic), "duplicate topic in list : {}", topic);
        }
    }
}
