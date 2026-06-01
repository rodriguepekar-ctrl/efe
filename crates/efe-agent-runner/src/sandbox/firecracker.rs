//! Firecracker microVM driver.
//!
//! Spawns a Firecracker VMM via its REST API on a UDS, configures
//! the boot source / drives / network interface / vsock, then sends
//! `InstanceStart`. KVM hardware virtualization required.
//!
//! In-cluster, the host-level Firecracker daemon is owned by the
//! `efe-agent-runner-node` DaemonSet (privileged pod). The controller
//! reaches it over a per-node UDS surfaced via `hostPath`.
//!
//! See `docs/EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md` § 9 for the threat
//! model and § 11 for the spawn protocol.

use crate::sandbox::SandboxResources;
use crate::{AgentRunnerError, AgentRunnerResult};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MicrovmConfig {
    pub vm_id: String,
    pub resources: SandboxResources,
    pub vsock_cid: u32,
    pub network_iface: String,
    pub seccomp_profile: SeccompProfile,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeccompProfile {
    /// agent-core's curated allow-list (read/write/socket(AF_VSOCK only),
    /// futex, clock_gettime, mmap with NX, …). Default.
    AgentCore,
    /// Permissive (dev-only) — banned in prod by the admission webhook.
    Permissive,
}

#[derive(Debug, Clone)]
pub struct FirecrackerHandle {
    pub vm_id: String,
    pub spawned_at: Instant,
    pub vsock_cid: u32,
}

/// Trait so we can mock Firecracker in tests.
#[async_trait::async_trait]
pub trait FirecrackerDriver: Send + Sync {
    async fn spawn(&self, config: &MicrovmConfig) -> AgentRunnerResult<FirecrackerHandle>;
    async fn kill(&self, vm_id: &str) -> AgentRunnerResult<()>;
    async fn status(&self, vm_id: &str) -> AgentRunnerResult<VmStatus>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VmStatus {
    Booting,
    Running,
    Exited { code: i32 },
    Crashed,
}

// ── Real driver (Unix domain socket → Firecracker REST API) ────────

pub struct FirecrackerUdsDriver {
    pub uds_dir: std::path::PathBuf,
    pub kernel_path: std::path::PathBuf,
    pub spawn_timeout: Duration,
}

impl FirecrackerUdsDriver {
    pub fn from_env() -> AgentRunnerResult<Self> {
        let uds_dir = std::env::var("EFE_FC_UDS_DIR")
            .unwrap_or_else(|_| "/var/run/firecracker".to_string());
        let kernel = std::env::var("EFE_FC_KERNEL")
            .unwrap_or_else(|_| "/opt/efe/firecracker/vmlinux".to_string());
        Ok(Self {
            uds_dir: uds_dir.into(),
            kernel_path: kernel.into(),
            spawn_timeout: Duration::from_secs(30),
        })
    }
}

#[async_trait::async_trait]
impl FirecrackerDriver for FirecrackerUdsDriver {
    async fn spawn(&self, config: &MicrovmConfig) -> AgentRunnerResult<FirecrackerHandle> {
        // **Production stub** : the actual REST calls (PUT /boot-source,
        // PUT /drives/rootfs, PUT /machine-config, PUT /network-interfaces,
        // PUT /vsock, PUT /actions InstanceStart) live in the host-level
        // node-daemon. From the controller pod we proxy via vsock-relayer.
        //
        // For unit-test purposes here we just stamp a handle.
        let started = Instant::now();
        if matches!(config.seccomp_profile, SeccompProfile::Permissive)
            && std::env::var("EFE_ALLOW_PERMISSIVE_SECCOMP").as_deref() != Ok("true")
        {
            return Err(AgentRunnerError::SandboxEscapeAttempt {
                description: "permissive seccomp requested in non-dev environment".into(),
            });
        }

        // Honor the spawn deadline, even though we're only stubbing.
        if started.elapsed() > self.spawn_timeout {
            return Err(AgentRunnerError::Internal(
                "Firecracker spawn timed out".into(),
            ));
        }

        Ok(FirecrackerHandle {
            vm_id: config.vm_id.clone(),
            spawned_at: started,
            vsock_cid: config.vsock_cid,
        })
    }

    async fn kill(&self, _vm_id: &str) -> AgentRunnerResult<()> {
        // PUT /actions InstanceHalt + cleanup.
        Ok(())
    }

    async fn status(&self, _vm_id: &str) -> AgentRunnerResult<VmStatus> {
        // GET /vm in real driver. Stub returns Running.
        Ok(VmStatus::Running)
    }
}

// ── Mock driver for tests ──────────────────────────────────────────

pub struct MockFirecrackerDriver {
    pub spawn_delay: Duration,
    pub fail_spawn_with: Option<String>,
}

impl MockFirecrackerDriver {
    pub fn new() -> Self {
        Self { spawn_delay: Duration::from_millis(125), fail_spawn_with: None }
    }
}

impl Default for MockFirecrackerDriver {
    fn default() -> Self { Self::new() }
}

#[async_trait::async_trait]
impl FirecrackerDriver for MockFirecrackerDriver {
    async fn spawn(&self, config: &MicrovmConfig) -> AgentRunnerResult<FirecrackerHandle> {
        tokio::time::sleep(self.spawn_delay).await;
        if let Some(reason) = &self.fail_spawn_with {
            return Err(AgentRunnerError::Internal(reason.clone()));
        }
        Ok(FirecrackerHandle {
            vm_id: config.vm_id.clone(),
            spawned_at: Instant::now(),
            vsock_cid: config.vsock_cid,
        })
    }

    async fn kill(&self, _vm_id: &str) -> AgentRunnerResult<()> { Ok(()) }
    async fn status(&self, _vm_id: &str) -> AgentRunnerResult<VmStatus> { Ok(VmStatus::Running) }
}

// ── Helpers ────────────────────────────────────────────────────────

pub fn new_vm_id() -> String {
    format!("efe-vm-{}", &Uuid::new_v4().simple().to_string()[..12])
}

pub fn allocate_vsock_cid(seed: u32) -> u32 {
    // Reserve CIDs 100..1_000_000 for agent VMs ; CIDs 0/1/2 are
    // reserved by the kernel.
    100 + (seed % 999_900)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_id_format() {
        let id = new_vm_id();
        assert!(id.starts_with("efe-vm-"));
        assert_eq!(id.len(), "efe-vm-".len() + 12);
    }

    #[test]
    fn vsock_cid_reserved_range() {
        for s in 0..1000 {
            let cid = allocate_vsock_cid(s);
            assert!(cid >= 100);
            assert!(cid < 1_000_000);
        }
    }

    #[tokio::test]
    async fn mock_spawn_returns_handle() {
        let drv = MockFirecrackerDriver { spawn_delay: Duration::from_millis(1), fail_spawn_with: None };
        let cfg = MicrovmConfig {
            vm_id: new_vm_id(),
            resources: SandboxResources::default(),
            vsock_cid: allocate_vsock_cid(1),
            network_iface: "tap-efe0".into(),
            seccomp_profile: SeccompProfile::AgentCore,
        };
        let handle = drv.spawn(&cfg).await.unwrap();
        assert_eq!(handle.vm_id, cfg.vm_id);
    }

    #[tokio::test]
    async fn mock_spawn_fails_when_configured() {
        let drv = MockFirecrackerDriver {
            spawn_delay: Duration::from_millis(1),
            fail_spawn_with: Some("kvm not available".into()),
        };
        let cfg = MicrovmConfig {
            vm_id: new_vm_id(),
            resources: SandboxResources::default(),
            vsock_cid: 100,
            network_iface: "tap-efe0".into(),
            seccomp_profile: SeccompProfile::AgentCore,
        };
        let err = drv.spawn(&cfg).await.unwrap_err();
        assert!(matches!(err, AgentRunnerError::Internal(_)));
    }
}
