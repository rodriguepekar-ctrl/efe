//! Production Firecracker driver — talks to the per-VM UDS socket via
//! `hyper` HTTP/1 client. Replaces the M25 `FirecrackerUdsDriver` stub.
//!
//! The boot sequence is :
//!
//!   1. PUT /boot-source     (kernel + boot args)
//!   2. PUT /drives/rootfs   (rootfs ext4 ro)
//!   3. PUT /machine-config  (vCPU + memory)
//!   4. PUT /network-interfaces/eth0   (TAP iface)
//!   5. PUT /vsock           (vsock CID)
//!   6. PUT /actions {"action_type":"InstanceStart"}
//!
//! Errors at any step trigger `kill(vm_id)` to reclaim partial state.

use crate::sandbox::firecracker::{
    FirecrackerDriver, FirecrackerHandle, MicrovmConfig, SeccompProfile, VmStatus,
};
use crate::{AgentRunnerError, AgentRunnerResult};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Production Firecracker driver. Talks to the per-VM UDS socket.
pub struct FirecrackerRealDriver {
    pub uds_dir: PathBuf,
    pub kernel_path: PathBuf,
    pub rootfs_dir: PathBuf,
    pub spawn_timeout: Duration,
    pub kill_grace: Duration,
}

impl FirecrackerRealDriver {
    pub fn from_env() -> AgentRunnerResult<Self> {
        let uds_dir = PathBuf::from(
            std::env::var("EFE_FC_UDS_DIR").unwrap_or_else(|_| "/var/run/firecracker".into()),
        );
        let kernel_path = PathBuf::from(
            std::env::var("EFE_FC_KERNEL")
                .unwrap_or_else(|_| "/opt/efe/firecracker/vmlinux".into()),
        );
        let rootfs_dir = PathBuf::from(
            std::env::var("EFE_FC_ROOTFS_DIR")
                .unwrap_or_else(|_| "/var/lib/efe/agent-runner/rootfs".into()),
        );
        Ok(Self {
            uds_dir,
            kernel_path,
            rootfs_dir,
            spawn_timeout: Duration::from_secs(30),
            kill_grace: Duration::from_secs(10),
        })
    }

    fn socket_path(&self, vm_id: &str) -> PathBuf {
        self.uds_dir.join(format!("{}.socket", vm_id))
    }

    fn rootfs_path(&self, vm_id: &str) -> PathBuf {
        self.rootfs_dir.join(format!("{}.ext4", vm_id))
    }

    /// PUT helper — issues an HTTP/1.1 PUT to the Firecracker UDS, body
    /// is JSON-serialized. Returns Ok on 2xx, error otherwise.
    async fn put<T: Serialize>(
        &self,
        vm_id: &str,
        path: &str,
        body: &T,
    ) -> AgentRunnerResult<()> {
        let started = Instant::now();
        let body_json = serde_json::to_vec(body)
            .map_err(|e| AgentRunnerError::Internal(format!("serialize : {}", e)))?;

        let socket = self.socket_path(vm_id);
        let result = tokio::time::timeout(
            self.spawn_timeout,
            put_uds(&socket, path, &body_json),
        )
        .await;

        let api_label = api_label(path);
        match result {
            Ok(Ok(status)) => {
                crate::metrics::AGENT_FIRECRACKER_API_DURATION
                    .with_label_values(&[api_label, &status.as_u16().to_string()])
                    .observe(started.elapsed().as_secs_f64());
                if !(200..300).contains(&status.as_u16()) {
                    return Err(AgentRunnerError::MicroVmProvisioningFailed(format!(
                        "PUT {} returned {}", path, status
                    )));
                }
                Ok(())
            }
            Ok(Err(e)) => {
                crate::metrics::AGENT_FIRECRACKER_API_DURATION
                    .with_label_values(&[api_label, "io_error"])
                    .observe(started.elapsed().as_secs_f64());
                Err(AgentRunnerError::MicroVmProvisioningFailed(format!(
                    "PUT {} : {}", path, e
                )))
            }
            Err(_) => {
                crate::metrics::AGENT_FIRECRACKER_API_DURATION
                    .with_label_values(&[api_label, "timeout"])
                    .observe(started.elapsed().as_secs_f64());
                Err(AgentRunnerError::MicroVmProvisioningFailed(format!(
                    "PUT {} timed out after {}s", path, self.spawn_timeout.as_secs()
                )))
            }
        }
    }
}

#[async_trait::async_trait]
impl FirecrackerDriver for FirecrackerRealDriver {
    async fn spawn(&self, config: &MicrovmConfig) -> AgentRunnerResult<FirecrackerHandle> {
        let started = Instant::now();

        if matches!(config.seccomp_profile, SeccompProfile::Permissive)
            && std::env::var("EFE_ALLOW_PERMISSIVE_SECCOMP").as_deref() != Ok("true")
        {
            return Err(AgentRunnerError::SandboxEscapeAttempt {
                description: "permissive seccomp requested in non-dev environment".into(),
            });
        }

        // 1. boot-source
        self.put(&config.vm_id, "/boot-source", &serde_json::json!({
            "kernel_image_path": self.kernel_path.display().to_string(),
            "boot_args": "console=ttyS0 reboot=k panic=1 pci=off random.trust_cpu=on init=/sbin/init",
        })).await?;

        // 2. rootfs
        let rootfs = self.rootfs_path(&config.vm_id);
        self.put(&config.vm_id, "/drives/rootfs", &serde_json::json!({
            "drive_id": "rootfs",
            "path_on_host": rootfs.display().to_string(),
            "is_root_device": true,
            "is_read_only": true,
        })).await?;

        // 3. machine-config
        self.put(&config.vm_id, "/machine-config", &serde_json::json!({
            "vcpu_count": config.resources.vcpus,
            "mem_size_mib": config.resources.memory_mib,
            "smt": false,
            "track_dirty_pages": false,
        })).await?;

        // 4. network-interfaces/eth0
        let mac = format!(
            "AA:FC:00:00:{:02X}:{:02X}",
            (config.vsock_cid >> 8) & 0xFF,
            config.vsock_cid & 0xFF,
        );
        self.put(&config.vm_id, "/network-interfaces/eth0", &serde_json::json!({
            "iface_id": "eth0",
            "host_dev_name": config.network_iface,
            "guest_mac": mac,
        })).await?;

        // 5. vsock
        self.put(&config.vm_id, "/vsock", &serde_json::json!({
            "vsock_id": "primary",
            "guest_cid": config.vsock_cid,
            "uds_path": format!("/var/run/efe-vsock-{}.sock", config.vm_id),
        })).await?;

        // 6. InstanceStart — note : explicit step ordering is critical.
        self.put(&config.vm_id, "/actions", &serde_json::json!({
            "action_type": "InstanceStart",
        })).await?;

        let took_ms = started.elapsed().as_millis() as u64;
        tracing::info!(
            vm_id = %config.vm_id,
            took_ms,
            mac,
            cid = config.vsock_cid,
            "Firecracker microVM started",
        );

        Ok(FirecrackerHandle {
            vm_id: config.vm_id.clone(),
            spawned_at: started,
            vsock_cid: config.vsock_cid,
        })
    }

    async fn kill(&self, vm_id: &str) -> AgentRunnerResult<()> {
        // Send SendCtrlAltDel for graceful shutdown ; then InstanceHalt.
        let _ = self.put(vm_id, "/actions", &serde_json::json!({
            "action_type": "SendCtrlAltDel",
        })).await;
        tokio::time::sleep(self.kill_grace).await;
        let _ = self.put(vm_id, "/actions", &serde_json::json!({
            "action_type": "InstanceHalt",
        })).await;
        // Cleanup UDS socket files.
        let _ = tokio::fs::remove_file(self.socket_path(vm_id)).await;
        let _ = tokio::fs::remove_file(format!("/var/run/efe-vsock-{}.sock", vm_id)).await;
        Ok(())
    }

    async fn status(&self, vm_id: &str) -> AgentRunnerResult<VmStatus> {
        // Probe by reading /vm endpoint via GET.
        let socket = self.socket_path(vm_id);
        match tokio::time::timeout(
            Duration::from_secs(2),
            get_uds(&socket, "/"),
        )
        .await
        {
            Ok(Ok(status)) if (200..300).contains(&status.as_u16()) => Ok(VmStatus::Running),
            _ => Ok(VmStatus::Crashed),
        }
    }
}

// ── Low-level UDS HTTP/1.1 client ────────────────────────────────────
//
// Avoids pulling hyper-util just for this. Firecracker speaks HTTP/1.1
// over UDS ; we hand-write the request line + parse the response line.

async fn put_uds(socket: &Path, path: &str, body: &[u8]) -> std::io::Result<http::StatusCode> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    let mut stream = UnixStream::connect(socket).await?;
    let req = format!(
        "PUT {} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        path,
        body.len(),
    );
    stream.write_all(req.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    parse_status(&buf)
}

async fn get_uds(socket: &Path, path: &str) -> std::io::Result<http::StatusCode> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    let mut stream = UnixStream::connect(socket).await?;
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        path,
    );
    stream.write_all(req.as_bytes()).await?;
    stream.flush().await?;

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    parse_status(&buf)
}

fn parse_status(buf: &[u8]) -> std::io::Result<http::StatusCode> {
    let s = std::str::from_utf8(buf).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("utf8 : {}", e))
    })?;
    let line = s.lines().next().unwrap_or("");
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 || !parts[0].starts_with("HTTP/") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("bad status line : {:?}", line),
        ));
    }
    let code: u16 = parts[1]
        .parse()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}", e)))?;
    http::StatusCode::from_u16(code).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("status : {}", e))
    })
}

fn api_label(path: &str) -> &'static str {
    match path {
        "/boot-source" => "boot_source",
        "/drives/rootfs" => "drives_rootfs",
        "/machine-config" => "machine_config",
        "/network-interfaces/eth0" => "network_iface",
        "/vsock" => "vsock",
        "/actions" => "actions",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_200() {
        let buf = b"HTTP/1.1 204 No Content\r\n\r\n";
        assert_eq!(parse_status(buf).unwrap(), http::StatusCode::NO_CONTENT);
    }

    #[test]
    fn parse_status_400() {
        let buf = b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n";
        assert_eq!(parse_status(buf).unwrap(), http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn parse_status_invalid() {
        assert!(parse_status(b"garbage").is_err());
    }

    #[test]
    fn api_label_dispatches() {
        assert_eq!(api_label("/boot-source"), "boot_source");
        assert_eq!(api_label("/actions"), "actions");
        assert_eq!(api_label("/unknown"), "other");
    }

    #[test]
    fn rootfs_path_layout() {
        let drv = FirecrackerRealDriver {
            uds_dir: PathBuf::from("/var/run/firecracker"),
            kernel_path: PathBuf::from("/opt/efe/firecracker/vmlinux"),
            rootfs_dir: PathBuf::from("/var/lib/efe/agent-runner/rootfs"),
            spawn_timeout: Duration::from_secs(30),
            kill_grace: Duration::from_secs(10),
        };
        assert_eq!(
            drv.rootfs_path("efe-vm-abc123").display().to_string(),
            "/var/lib/efe/agent-runner/rootfs/efe-vm-abc123.ext4"
        );
        assert_eq!(
            drv.socket_path("efe-vm-abc123").display().to_string(),
            "/var/run/firecracker/efe-vm-abc123.socket"
        );
    }
}
