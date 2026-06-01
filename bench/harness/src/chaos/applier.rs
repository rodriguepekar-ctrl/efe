//! PR #21 § 17.2 — chaos-mesh applier (kubectl-driven + mock).

use std::path::Path;

use async_trait::async_trait;

use crate::error::{BenchError, BenchResult};

/// Apply / delete chaos faults.  Two impls : real (kubectl
/// subprocess) + mock (test-only).
#[async_trait]
pub trait ChaosApplier: Send + Sync {
    async fn apply_fault(&self, fault_id: &str, crd_path: &Path, namespace: &str) -> BenchResult<()>;
    async fn delete_fault(&self, fault_id: &str, namespace: &str) -> BenchResult<()>;
}

/// Real applier — runs `kubectl apply -f` + `kubectl delete -f`.
pub struct KubectlApplier {
    kubeconfig: std::path::PathBuf,
}

impl KubectlApplier {
    #[must_use]
    pub const fn new(kubeconfig: std::path::PathBuf) -> Self {
        Self { kubeconfig }
    }
}

#[async_trait]
impl ChaosApplier for KubectlApplier {
    async fn apply_fault(
        &self,
        _fault_id: &str,
        crd_path: &Path,
        namespace: &str,
    ) -> BenchResult<()> {
        let out = tokio::process::Command::new("kubectl")
            .env("KUBECONFIG", &self.kubeconfig)
            .args([
                "apply",
                "-f",
                &crd_path.to_string_lossy(),
                "-n",
                namespace,
            ])
            .output()
            .await
            .map_err(|e| BenchError::ChaosApply(format!("kubectl spawn : {e}")))?;
        if !out.status.success() {
            return Err(BenchError::ChaosApply(format!(
                "kubectl apply -f {} : {}",
                crd_path.display(),
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(())
    }

    async fn delete_fault(&self, fault_id: &str, namespace: &str) -> BenchResult<()> {
        // Delete by name (the CRD's metadata.name should match the
        // fault_id ; the scenario YAML enforces this convention).
        let out = tokio::process::Command::new("kubectl")
            .env("KUBECONFIG", &self.kubeconfig)
            .args([
                "delete",
                "podchaos,networkchaos,stresschaos",
                fault_id,
                "-n",
                namespace,
                "--ignore-not-found",
            ])
            .output()
            .await
            .map_err(|e| BenchError::ChaosApply(format!("kubectl spawn : {e}")))?;
        if !out.status.success() {
            // Best-effort delete : warn but don't abort.
            tracing::warn!(
                stderr = %String::from_utf8_lossy(&out.stderr),
                "kubectl delete chaos non-zero"
            );
        }
        Ok(())
    }
}

/// Mock applier — used in tests + dry-run mode.  Captures the
/// applied/deleted calls in-memory.
#[derive(Default)]
pub struct MockApplier {
    pub applied: parking_lot::Mutex<Vec<(String, std::path::PathBuf, String)>>,
    pub deleted: parking_lot::Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl ChaosApplier for MockApplier {
    async fn apply_fault(
        &self,
        fault_id: &str,
        crd_path: &Path,
        namespace: &str,
    ) -> BenchResult<()> {
        self.applied.lock().push((
            fault_id.into(),
            crd_path.to_path_buf(),
            namespace.into(),
        ));
        Ok(())
    }

    async fn delete_fault(&self, fault_id: &str, namespace: &str) -> BenchResult<()> {
        self.deleted.lock().push((fault_id.into(), namespace.into()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_records_apply_and_delete() {
        let m = MockApplier::default();
        m.apply_fault("f1", Path::new("/tmp/x.yaml"), "ns1").await.unwrap();
        m.delete_fault("f1", "ns1").await.unwrap();
        assert_eq!(m.applied.lock().len(), 1);
        assert_eq!(m.deleted.lock().len(), 1);
        assert_eq!(m.applied.lock()[0].0, "f1");
        assert_eq!(m.applied.lock()[0].2, "ns1");
    }
}
