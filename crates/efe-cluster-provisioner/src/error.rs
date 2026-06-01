//! PR #23 § 10 — error taxonomy for the cluster-provisioner crate.

use thiserror::Error;

pub type ProvisionResult<T> = Result<T, ProvisionError>;

/// Errors emitted by provisioner implementations.
///
/// Consumed by both PR #21 (bench-harness) and PR #23 (smoke) via
/// `From<ProvisionError>` impls on their respective error types.
#[derive(Debug, Error)]
pub enum ProvisionError {
    /// Generic provisioner failure (kind / k3s / external).
    #[error("{provisioner} provisioner : {reason}")]
    Cluster { provisioner: String, reason: String },

    /// `kubectl` invocation failed.
    #[error("kubectl {command} failed : {stderr}")]
    Kubectl { command: String, stderr: String },

    /// I/O failure (e.g. kubeconfig write).
    #[error("io : {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cluster_error_renders_with_provisioner_name() {
        let e = ProvisionError::Cluster {
            provisioner: "kind".into(),
            reason: "boom".into(),
        };
        let s = format!("{e}");
        assert!(s.contains("kind"));
        assert!(s.contains("boom"));
    }

    #[test]
    fn kubectl_error_renders() {
        let e = ProvisionError::Kubectl {
            command: "get pods".into(),
            stderr: "no auth".into(),
        };
        let s = format!("{e}");
        assert!(s.contains("get pods"));
        assert!(s.contains("no auth"));
    }
}
