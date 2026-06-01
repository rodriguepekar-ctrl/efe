//! `efe-cluster-provisioner` — cluster provisioning abstraction
//! shared between PR #21 bench-harness and PR #23 smoke.
//!
//! **Stability contract** : STABILITY_CONTRACT v1.5 § 23.6.  Trait
//! surface frozen for ≥ 6 chart-minor versions ; adding methods is
//! breaking, only adding default-impl methods is non-breaking.
//!
//! ## Usage
//!
//! ```no_run
//! # async fn doc() -> Result<(), efe_cluster_provisioner::ProvisionError> {
//! use efe_cluster_provisioner::{KindProvisioner, ClusterProvisioner};
//! let p = KindProvisioner::new("my-cluster".into());
//! let kubeconfig = p.provision().await?;
//! println!("kubeconfig at {}", kubeconfig.display());
//! p.teardown().await?;
//! # Ok(()) }
//! ```

#![warn(rust_2018_idioms, clippy::all)]
#![allow(clippy::module_name_repetitions, missing_docs)]

pub mod compat;
pub mod error;
pub mod external;
pub mod k3s;
pub mod kind;
pub mod traits;

pub use crate::error::{ProvisionError, ProvisionResult};
pub use crate::external::ExternalProvisioner;
pub use crate::k3s::K3sProvisioner;
pub use crate::kind::KindProvisioner;
pub use crate::traits::{ClusterProvisioner, ProvisionerKind};

use std::path::PathBuf;

/// Build a provisioner from a kind discriminator + an optional
/// kubeconfig (used only for `external`).  Mirrors PR #21
/// `bench/harness/src/cluster/mod.rs::from_cli`.
#[must_use]
pub fn from_kind(
    kind: ProvisionerKind,
    kubeconfig: Option<PathBuf>,
    cluster_name: String,
) -> Box<dyn ClusterProvisioner> {
    match kind {
        ProvisionerKind::Kind => Box::new(KindProvisioner::new(cluster_name)),
        ProvisionerKind::K3s => Box::new(K3sProvisioner::new(cluster_name)),
        ProvisionerKind::External => Box::new(ExternalProvisioner::new(
            kubeconfig.unwrap_or_default(),
        )),
    }
}

/// Helper : ensure `kubectl` is on PATH.  Returns a friendly error
/// when missing.
pub fn ensure_kubectl_available() -> ProvisionResult<()> {
    use std::process::Command;
    match Command::new("kubectl").arg("version").arg("--client=true").output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(ProvisionError::Kubectl {
            command: "version".into(),
            stderr: String::from_utf8_lossy(&o.stderr).to_string(),
        }),
        Err(e) => Err(ProvisionError::Cluster {
            provisioner: "system".into(),
            reason: format!("kubectl not on PATH : {e}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_kind_builds_kind() {
        let p = from_kind(ProvisionerKind::Kind, None, "x".into());
        assert_eq!(p.name(), "kind");
        assert_eq!(p.kind(), ProvisionerKind::Kind);
    }

    #[test]
    fn from_kind_builds_external() {
        let p = from_kind(ProvisionerKind::External, Some(PathBuf::from("/tmp/kc")), "x".into());
        assert_eq!(p.name(), "external");
    }

    #[test]
    fn from_kind_builds_k3s() {
        let p = from_kind(ProvisionerKind::K3s, None, "x".into());
        assert_eq!(p.name(), "k3s");
    }
}
