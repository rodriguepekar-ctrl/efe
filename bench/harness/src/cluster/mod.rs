//! PR #21 § 16 — cluster provisioning **compat shim** for PR #23 § 10
//! extraction.
//!
//! The full implementation now lives in
//! [`efe_cluster_provisioner`] (a workspace crate at
//! `crates/efe-cluster-provisioner/`).  This module re-exports the
//! public surface so PR #21 callers compile byte-identically (AC6).
//!
//! ⚠ Do **not** add new logic here.  Modify the shared crate instead.
//! New behaviour landing in this shim is a contract violation
//! (STABILITY_CONTRACT v1.5 § 23.6).

use std::path::PathBuf;

use crate::cli::ProvisionerKind as CliProvisionerKind;
use crate::error::{BenchError, BenchResult};

// ── Re-export the new crate's surface under the historical
//    `cluster::*` namespace.
pub use efe_cluster_provisioner::{
    ClusterProvisioner,
    ExternalProvisioner,
    K3sProvisioner,
    KindProvisioner,
    ProvisionerKind,
};

/// Map the bench-harness CLI's [`CliProvisionerKind`] to the shared
/// crate's [`ProvisionerKind`].
#[must_use]
const fn map_kind(k: CliProvisionerKind) -> ProvisionerKind {
    match k {
        CliProvisionerKind::Kind => ProvisionerKind::Kind,
        CliProvisionerKind::K3s => ProvisionerKind::K3s,
        CliProvisionerKind::External => ProvisionerKind::External,
    }
}

/// Build a provisioner from the bench-harness CLI args.
///
/// Same signature as the pre-extraction `from_cli` (PR #21 § 16) so
/// existing callers compile byte-identically.
#[must_use]
pub fn from_cli(
    kind: CliProvisionerKind,
    kubeconfig: Option<PathBuf>,
    cluster_name: String,
) -> Box<dyn ClusterProvisioner> {
    efe_cluster_provisioner::from_kind(map_kind(kind), kubeconfig, cluster_name)
}

/// Re-exported helper — `kubectl` PATH check.
pub fn ensure_kubectl_available() -> BenchResult<()> {
    efe_cluster_provisioner::ensure_kubectl_available().map_err(BenchError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_kind_preserves_taxonomy() {
        assert_eq!(map_kind(CliProvisionerKind::Kind), ProvisionerKind::Kind);
        assert_eq!(map_kind(CliProvisionerKind::K3s), ProvisionerKind::K3s);
        assert_eq!(map_kind(CliProvisionerKind::External), ProvisionerKind::External);
    }

    #[test]
    fn from_cli_builds_kind_provisioner() {
        let p = from_cli(CliProvisionerKind::Kind, None, "test".into());
        assert_eq!(p.name(), "kind");
    }

    #[test]
    fn from_cli_builds_external_provisioner() {
        let p = from_cli(CliProvisionerKind::External, Some(PathBuf::from("/tmp/kc")), "test".into());
        assert_eq!(p.name(), "external");
    }
}
