//! PR #23 § 10 — kind-based cluster provisioner (extracted from
//! PR #21 § 16.1).
//!
//! Spawns `kind create cluster --name <name>` ; tears down on Drop or
//! explicit `teardown()`.

use std::path::PathBuf;
use std::process::Command;

use async_trait::async_trait;
use tracing::{info, warn};

use crate::error::{ProvisionError, ProvisionResult};
use crate::traits::{ClusterProvisioner, ProvisionerKind};

pub struct KindProvisioner {
    cluster_name: String,
    image: String,
}

impl KindProvisioner {
    /// Default image is tested against EFE Phase-2 chart `v0.6.x`.
    /// Operators may override via [`Self::with_image`].
    #[must_use]
    pub fn new(cluster_name: String) -> Self {
        Self {
            cluster_name,
            image: "kindest/node:v1.30.0".into(),
        }
    }

    #[must_use]
    pub fn with_image(mut self, image: String) -> Self {
        self.image = image;
        self
    }

    /// Inspect the configured kind node image (used by smoke S1 boot
    /// diagnostics).
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }

    /// Inspect the configured cluster name.
    #[must_use]
    pub fn cluster_name(&self) -> &str {
        &self.cluster_name
    }
}

#[async_trait]
impl ClusterProvisioner for KindProvisioner {
    async fn provision(&self) -> ProvisionResult<PathBuf> {
        info!(name = %self.cluster_name, image = %self.image, "provisioning kind cluster");
        let out = Command::new("kind")
            .args([
                "create",
                "cluster",
                "--name",
                &self.cluster_name,
                "--image",
                &self.image,
                "--wait",
                "5m",
            ])
            .output()
            .map_err(|e| ProvisionError::Cluster {
                provisioner: "kind".into(),
                reason: format!("kind binary not on PATH : {e}"),
            })?;
        if !out.status.success() {
            return Err(ProvisionError::Cluster {
                provisioner: "kind".into(),
                reason: String::from_utf8_lossy(&out.stderr).to_string(),
            });
        }
        // Resolve kubeconfig path.
        let kc = Command::new("kind")
            .args(["get", "kubeconfig", "--name", &self.cluster_name])
            .output()
            .map_err(|e| ProvisionError::Cluster {
                provisioner: "kind".into(),
                reason: format!("kind get kubeconfig : {e}"),
            })?;
        if !kc.status.success() {
            return Err(ProvisionError::Cluster {
                provisioner: "kind".into(),
                reason: String::from_utf8_lossy(&kc.stderr).to_string(),
            });
        }
        // Write to a temp file (we don't log the contents — T90 from PR #21).
        let kc_path = std::env::temp_dir().join(format!("kind-{}.kubeconfig", self.cluster_name));
        std::fs::write(&kc_path, &kc.stdout)?;
        Ok(kc_path)
    }

    async fn teardown(&self) -> ProvisionResult<()> {
        let out = Command::new("kind")
            .args(["delete", "cluster", "--name", &self.cluster_name])
            .output();
        match out {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => {
                warn!(
                    stderr = %String::from_utf8_lossy(&o.stderr),
                    "kind delete cluster non-zero (best-effort)"
                );
                Ok(())
            }
            Err(e) => {
                warn!(error = %e, "kind delete binary not found ; best-effort");
                Ok(())
            }
        }
    }

    fn name(&self) -> &'static str {
        "kind"
    }

    fn kind(&self) -> ProvisionerKind {
        ProvisionerKind::Kind
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_default_image_v1_30() {
        let p = KindProvisioner::new("test".into());
        assert!(p.image.contains("v1.30"));
    }

    #[test]
    fn kind_with_image_override() {
        let p = KindProvisioner::new("test".into()).with_image("kindest/node:v1.29.0".into());
        assert!(p.image.contains("v1.29"));
    }

    #[test]
    fn kind_name_and_kind_discriminator_consistent() {
        let p = KindProvisioner::new("x".into());
        assert_eq!(p.name(), "kind");
        assert_eq!(p.kind(), ProvisionerKind::Kind);
    }
}
