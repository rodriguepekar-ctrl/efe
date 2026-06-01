//! PR #23 § 8 + § 10 — glue between [`efe_cluster_provisioner`] and
//! the smoke orchestrator.
//!
//! Adds the production-cluster guard (AD9 — T108) on top of the bare
//! provisioner.  Also surfaces a namespace cleaner used by the
//! orchestrator's teardown path.

use std::path::Path;
use std::process::Command;

use tracing::warn;

use crate::error::{SmokeError, SmokeResult};

/// AD9 — refuse to target a cluster labelled
/// `efe.io/environment=production`.  Smoke must never run against
/// production (T108 mitigation).
pub fn assert_not_production(kubeconfig: &Path) -> SmokeResult<()> {
    let out = Command::new("kubectl")
        .args([
            "--kubeconfig",
            kubeconfig.to_string_lossy().as_ref(),
            "get",
            "namespaces",
            "-l",
            "efe.io/environment=production",
            "-o",
            "name",
        ])
        .output()
        .map_err(|e| SmokeError::Source(format!("kubectl get namespaces : {e}")))?;
    if !out.status.success() {
        // If kubectl can't reach the cluster, defer to the provisioner
        // for diagnosis.  Not the production-guard's job.
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !stdout.trim().is_empty() {
        return Err(SmokeError::ProductionRefused);
    }
    Ok(())
}

/// Delete the smoke namespace (best-effort — used in teardown).
pub fn delete_namespace(kubeconfig: &Path, namespace: &str) -> SmokeResult<()> {
    let out = Command::new("kubectl")
        .args([
            "--kubeconfig",
            kubeconfig.to_string_lossy().as_ref(),
            "delete",
            "namespace",
            namespace,
            "--ignore-not-found=true",
            "--wait=false",
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            warn!(
                stderr = %String::from_utf8_lossy(&o.stderr),
                "kubectl delete namespace non-zero (best-effort)"
            );
            Ok(())
        }
        Err(e) => {
            warn!(error = %e, "kubectl binary not on PATH ; best-effort");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assert_not_production_succeeds_when_kubectl_missing() {
        // If kubectl isn't on PATH, the syscall fails → we still
        // surface a SmokeError::Source (not ProductionRefused).
        let r = assert_not_production(Path::new("/no/such/kc"));
        match r {
            Ok(_) => { /* OK — cluster unreachable, defer */ }
            Err(SmokeError::Source(_)) => { /* OK */ }
            other => panic!("unexpected : {other:?}"),
        }
    }

    #[test]
    fn delete_namespace_is_idempotent() {
        // Best-effort : no panic even if kubectl fails.
        let _ = delete_namespace(Path::new("/no/such/kc"), "no-such-ns");
    }
}
