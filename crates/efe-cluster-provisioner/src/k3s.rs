//! PR #23 § 10 — k3s provisioner (Phase-3 stub, extracted from PR #21 § 16.2).

use std::path::PathBuf;

use async_trait::async_trait;

use crate::error::{ProvisionError, ProvisionResult};
use crate::traits::{ClusterProvisioner, ProvisionerKind};

pub struct K3sProvisioner {
    cluster_name: String,
}

impl K3sProvisioner {
    #[must_use]
    pub fn new(cluster_name: String) -> Self {
        Self { cluster_name }
    }
}

#[async_trait]
impl ClusterProvisioner for K3sProvisioner {
    async fn provision(&self) -> ProvisionResult<PathBuf> {
        Err(ProvisionError::Cluster {
            provisioner: "k3s".into(),
            reason: format!(
                "k3s provisioner is Phase-3 (Appendix G.1) ; use --provisioner=kind or external for cluster '{}'",
                self.cluster_name
            ),
        })
    }

    async fn teardown(&self) -> ProvisionResult<()> {
        Ok(())
    }

    fn name(&self) -> &'static str {
        "k3s"
    }

    fn kind(&self) -> ProvisionerKind {
        ProvisionerKind::K3s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn k3s_provision_returns_phase3_stub_error() {
        let p = K3sProvisioner::new("x".into());
        let err = p.provision().await.unwrap_err();
        match err {
            ProvisionError::Cluster { provisioner, .. } => assert_eq!(provisioner, "k3s"),
            other => panic!("expected ProvisionError::Cluster, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn k3s_teardown_is_noop() {
        let p = K3sProvisioner::new("x".into());
        p.teardown().await.unwrap();
    }
}
