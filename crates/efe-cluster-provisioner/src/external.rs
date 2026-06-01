//! PR #23 § 10 — external-kubeconfig provisioner (extracted from PR #21 § 16.3).
//!
//! Operator supplies a working kubeconfig ; harness does no
//! provision / teardown.

use std::path::PathBuf;

use async_trait::async_trait;

use crate::error::{ProvisionError, ProvisionResult};
use crate::traits::{ClusterProvisioner, ProvisionerKind};

pub struct ExternalProvisioner {
    kubeconfig: PathBuf,
}

impl ExternalProvisioner {
    #[must_use]
    pub const fn new(kubeconfig: PathBuf) -> Self {
        Self { kubeconfig }
    }

    /// Inspect the configured kubeconfig path (used by smoke S1 boot
    /// diagnostics).  Note : the **content** is never read or logged
    /// (AD31 from PR #21).
    #[must_use]
    pub fn kubeconfig_path(&self) -> &PathBuf {
        &self.kubeconfig
    }
}

#[async_trait]
impl ClusterProvisioner for ExternalProvisioner {
    async fn provision(&self) -> ProvisionResult<PathBuf> {
        if !self.kubeconfig.exists() {
            return Err(ProvisionError::Cluster {
                provisioner: "external".into(),
                reason: format!("kubeconfig {} does not exist", self.kubeconfig.display()),
            });
        }
        // AD31 — never read or log cert contents.
        Ok(self.kubeconfig.clone())
    }

    async fn teardown(&self) -> ProvisionResult<()> {
        Ok(())
    }

    fn name(&self) -> &'static str {
        "external"
    }

    fn kind(&self) -> ProvisionerKind {
        ProvisionerKind::External
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn provision_returns_existing_path() {
        let f = NamedTempFile::new().unwrap();
        let p = ExternalProvisioner::new(f.path().to_path_buf());
        let kc = p.provision().await.unwrap();
        assert_eq!(kc, f.path());
    }

    #[tokio::test]
    async fn provision_rejects_missing_kubeconfig() {
        let p = ExternalProvisioner::new(PathBuf::from("/nonexistent/kc-1234-x"));
        let err = p.provision().await.unwrap_err();
        match err {
            ProvisionError::Cluster { provisioner, .. } => assert_eq!(provisioner, "external"),
            other => panic!("expected ProvisionError::Cluster, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn external_teardown_is_noop() {
        let f = NamedTempFile::new().unwrap();
        let p = ExternalProvisioner::new(f.path().to_path_buf());
        p.teardown().await.unwrap();
    }
}
