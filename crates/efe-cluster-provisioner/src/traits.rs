//! PR #23 § 10 + STABILITY_CONTRACT v1.5 § 23.6 — frozen trait surface.
//!
//! ⚠ Adding a method here is a **major-version-bumping** change.
//! Only adding default-impl methods is non-breaking.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::ProvisionResult;

/// Common interface for cluster provisioners.
///
/// **Stability** : STABILITY_CONTRACT v1.5 § 23.6 (PR #23) — frozen
/// for ≥ 6 chart-minor versions.
#[async_trait]
pub trait ClusterProvisioner: Send + Sync {
    /// Provision the cluster.  Returns the path to a working
    /// kubeconfig.  Idempotent : if a cluster with the same name
    /// already exists, return its kubeconfig.
    async fn provision(&self) -> ProvisionResult<PathBuf>;

    /// Tear down the cluster.  Idempotent : safe to call on success
    /// or failure (best-effort log warning on errors).
    async fn teardown(&self) -> ProvisionResult<()>;

    /// Friendly name for logs + manifest labels.  Frozen values :
    /// `"kind" | "k3s" | "external"`.
    fn name(&self) -> &'static str;

    /// Discriminator (programmatic equivalent of `name()`).
    fn kind(&self) -> ProvisionerKind;
}

/// Provisioner-kind discriminator.  Frozen v1.5 § 23.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionerKind {
    Kind,
    K3s,
    External,
}

impl ProvisionerKind {
    /// Frozen string form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kind => "kind",
            Self::K3s => "k3s",
            Self::External => "external",
        }
    }

    /// Parse from the frozen string form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "kind" => Some(Self::Kind),
            "k3s" => Some(Self::K3s),
            "external" => Some(Self::External),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discriminator_string_form_frozen() {
        // STABILITY_CONTRACT v1.5 § 23.6.
        assert_eq!(ProvisionerKind::Kind.as_str(), "kind");
        assert_eq!(ProvisionerKind::K3s.as_str(), "k3s");
        assert_eq!(ProvisionerKind::External.as_str(), "external");
    }

    #[test]
    fn discriminator_roundtrips() {
        for k in [ProvisionerKind::Kind, ProvisionerKind::K3s, ProvisionerKind::External] {
            assert_eq!(ProvisionerKind::parse(k.as_str()), Some(k));
        }
    }

    #[test]
    fn discriminator_rejects_unknown() {
        assert!(ProvisionerKind::parse("microk8s").is_none());
        assert!(ProvisionerKind::parse("").is_none());
    }
}
