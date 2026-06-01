//! **Sprint 8 — N11 days 11-12** : Firecracker control-plane HTTP client.
//!
//! Firecracker exposes a REST-like API over a **Unix-domain socket**.
//! The host configures the VM (boot source, drives, machine config,
//! vsock device) via PUT requests + starts the VM via a POST.
//!
//! # Why hand-rolled HTTP/1.1 instead of a crate
//!
//! - No new workspace deps (would require updating Cargo.toml).
//! - The API surface is **5 endpoints**, all PUT with JSON bodies.
//! - Firecracker's HTTP server is minimal — single-request connections,
//!   no chunked transfer, no compression.  Raw HTTP/1.1 is ~ 100 LOC.
//! - Production-grade alternative : swap in `hyperlocal` + `hyper` in
//!   Sprint 9 if we need keep-alive or streaming.

#![cfg(target_os = "linux")]

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

use super::config::KindConfig;
use super::types::VsockCid;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("connect to {path} failed : {reason}")]
    Connect { path: PathBuf, reason: String },

    #[error("I/O error : {0}")]
    Io(String),

    #[error("firecracker rejected request : status={status} body={body}")]
    Rejected { status: u16, body: String },

    #[error("malformed HTTP response : {0}")]
    MalformedResponse(String),

    #[error("JSON encode error : {0}")]
    JsonEncode(String),

    #[error("timeout after {0:?}")]
    Timeout(Duration),
}

// ─────────────────────────────────────────────────────────────────────
//  Firecracker API DTOs
// ─────────────────────────────────────────────────────────────────────

/// Firecracker `PUT /machine-config` body.
#[derive(Debug, Serialize, Deserialize)]
struct MachineConfigDto<'a> {
    vcpu_count: u32,
    mem_size_mib: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    smt: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    track_dirty_pages: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu_template: Option<&'a str>,
}

/// Firecracker `PUT /boot-source` body.
#[derive(Debug, Serialize, Deserialize)]
struct BootSourceDto<'a> {
    kernel_image_path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    boot_args: Option<&'a str>,
}

/// Firecracker `PUT /drives/<drive_id>` body.
#[derive(Debug, Serialize, Deserialize)]
struct DriveDto<'a> {
    drive_id: &'a str,
    path_on_host: &'a str,
    is_root_device: bool,
    is_read_only: bool,
}

/// Firecracker `PUT /vsock` body.
#[derive(Debug, Serialize, Deserialize)]
struct VsockDto<'a> {
    guest_cid: u32,
    uds_path: &'a str,
    vsock_id: &'a str,
}

/// Firecracker `PUT /actions` body.
#[derive(Debug, Serialize, Deserialize)]
struct ActionDto<'a> {
    action_type: &'a str,
}

// ─────────────────────────────────────────────────────────────────────
//  ApiClient
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : Firecracker control-plane client.
///
/// One client per VM.  Owned by the lifecycle controller during
/// spawn ; dropped once the VM is warm + serving on vsock.
///
/// Note : the *inference* RPC uses vsock, NOT this HTTP API.  The HTTP
/// API is purely for VM provisioning (config + start).  Post-start,
/// the API socket is closed.
#[derive(Debug)]
pub struct ApiClient {
    socket_path: PathBuf,
    timeout: Duration,
}

impl ApiClient {
    pub fn new(socket_path: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self {
            socket_path: socket_path.into(),
            timeout,
        }
    }

    /// Configure CPU + memory for the VM.
    pub fn put_machine_config(&self, kind: &KindConfig) -> Result<(), ApiError> {
        let body = MachineConfigDto {
            vcpu_count: kind.vcpus,
            mem_size_mib: kind.memory_mb,
            smt: Some(false),
            track_dirty_pages: Some(false),
            cpu_template: None,
        };
        self.put_json("/machine-config", &body)
    }

    /// Set the kernel image + cmdline.
    pub fn put_boot_source(
        &self,
        kernel_image: &Path,
        boot_args: Option<&str>,
    ) -> Result<(), ApiError> {
        let body = BootSourceDto {
            kernel_image_path: kernel_image
                .to_str()
                .ok_or_else(|| ApiError::Io("non-utf8 kernel path".into()))?,
            boot_args,
        };
        self.put_json("/boot-source", &body)
    }

    /// Mount the rootfs as drive `rootfs`.
    pub fn put_rootfs(&self, rootfs: &Path) -> Result<(), ApiError> {
        let body = DriveDto {
            drive_id: "rootfs",
            path_on_host: rootfs
                .to_str()
                .ok_or_else(|| ApiError::Io("non-utf8 rootfs path".into()))?,
            is_root_device: true,
            is_read_only: true,
        };
        self.put_json("/drives/rootfs", &body)
    }

    /// Attach a vsock device.  `uds_path` is the host-side Unix-domain
    /// socket that the guest's vsock will be mapped onto (`/var/run/efe-nano/vsock/<vm_id>.sock`).
    pub fn put_vsock(&self, cid: VsockCid, uds_path: &Path) -> Result<(), ApiError> {
        let body = VsockDto {
            guest_cid: cid.raw(),
            uds_path: uds_path
                .to_str()
                .ok_or_else(|| ApiError::Io("non-utf8 vsock path".into()))?,
            vsock_id: "primary",
        };
        self.put_json("/vsock", &body)
    }

    /// `PUT /actions { "action_type": "InstanceStart" }`.  Boots the VM.
    pub fn instance_start(&self) -> Result<(), ApiError> {
        let body = ActionDto {
            action_type: "InstanceStart",
        };
        self.put_json("/actions", &body)
    }

    /// `PUT /actions { "action_type": "SendCtrlAltDel" }`.  Sends a
    /// "graceful shutdown" signal to the guest kernel.  The guest's
    /// systemd / OpenRC handler (usually wired to `/sbin/poweroff`)
    /// then exits cleanly.
    pub fn graceful_shutdown(&self) -> Result<(), ApiError> {
        let body = ActionDto {
            action_type: "SendCtrlAltDel",
        };
        self.put_json("/actions", &body)
    }

    // ───────────────────────────────────────────────────────────────
    //  HTTP/1.1 over Unix socket
    // ───────────────────────────────────────────────────────────────

    fn put_json<T: Serialize>(&self, path: &str, body: &T) -> Result<(), ApiError> {
        let json = serde_json::to_vec(body).map_err(|e| ApiError::JsonEncode(e.to_string()))?;
        self.send_request("PUT", path, &json)
    }

    fn send_request(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> Result<(), ApiError> {
        let mut stream = UnixStream::connect(&self.socket_path).map_err(|e| ApiError::Connect {
            path: self.socket_path.clone(),
            reason: e.to_string(),
        })?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|e| ApiError::Io(e.to_string()))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|e| ApiError::Io(e.to_string()))?;

        // Build HTTP/1.1 request.
        let request_line = format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: localhost\r\n\
             Accept: application/json\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(request_line.as_bytes())
            .map_err(|e| ApiError::Io(format!("write head : {e}")))?;
        stream
            .write_all(body)
            .map_err(|e| ApiError::Io(format!("write body : {e}")))?;
        stream
            .flush()
            .map_err(|e| ApiError::Io(format!("flush : {e}")))?;

        // Read entire response (Firecracker uses short responses).
        let mut buf = Vec::with_capacity(4096);
        stream
            .read_to_end(&mut buf)
            .map_err(|e| ApiError::Io(format!("read : {e}")))?;
        parse_response(&buf)
    }
}

/// Parse a minimal HTTP/1.1 response.  Firecracker returns either
/// 204 No Content on success or 400/500-class on error with a JSON
/// body explaining.
fn parse_response(buf: &[u8]) -> Result<(), ApiError> {
    let text = std::str::from_utf8(buf)
        .map_err(|e| ApiError::MalformedResponse(format!("non-utf8 : {e}")))?;
    let mut lines = text.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| ApiError::MalformedResponse("empty response".into()))?;
    // status_line : "HTTP/1.1 <status> <reason>"
    let mut parts = status_line.split_whitespace();
    let _proto = parts.next();
    let status: u16 = parts
        .next()
        .ok_or_else(|| ApiError::MalformedResponse("no status".into()))?
        .parse()
        .map_err(|_| ApiError::MalformedResponse(format!("bad status : {status_line}")))?;

    if (200..300).contains(&status) {
        Ok(())
    } else {
        // Find body (after empty line).
        let body_start = text.find("\r\n\r\n").map(|i| i + 4).unwrap_or(text.len());
        let body = &text[body_start..];
        Err(ApiError::Rejected {
            status,
            body: body.trim().to_string(),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests — exercise the parser without a real Firecracker
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_204_no_content_is_ok() {
        let resp = b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n";
        parse_response(resp).unwrap();
    }

    #[test]
    fn parse_200_ok_is_ok() {
        let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        parse_response(resp).unwrap();
    }

    #[test]
    fn parse_400_returns_rejected_with_body() {
        let resp = b"HTTP/1.1 400 Bad Request\r\nContent-Length: 26\r\n\r\n{\"fault_message\":\"oops\"}";
        let err = parse_response(resp).unwrap_err();
        match err {
            ApiError::Rejected { status, body } => {
                assert_eq!(status, 400);
                assert!(body.contains("oops"));
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn parse_500_returns_rejected() {
        let resp = b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
        let err = parse_response(resp).unwrap_err();
        assert!(matches!(err, ApiError::Rejected { status: 500, .. }));
    }

    #[test]
    fn parse_malformed_status_line_errors() {
        let resp = b"GIBBERISH\r\n\r\n";
        let err = parse_response(resp).unwrap_err();
        assert!(matches!(err, ApiError::MalformedResponse(_)));
    }

    #[test]
    fn parse_non_utf8_errors() {
        let resp = &[0xFF_u8, 0xFE, 0xFD];
        let err = parse_response(resp).unwrap_err();
        assert!(matches!(err, ApiError::MalformedResponse(_)));
    }

    #[test]
    fn parse_empty_buffer_errors() {
        let err = parse_response(b"").unwrap_err();
        assert!(matches!(err, ApiError::MalformedResponse(_)));
    }

    /// Serialization of MachineConfigDto must match Firecracker's API
    /// schema exactly.  Specs : https://github.com/firecracker-microvm/firecracker/blob/main/src/api_server/swagger/firecracker.yaml
    #[test]
    fn machine_config_serializes_with_correct_keys() {
        let cfg = MachineConfigDto {
            vcpu_count: 2,
            mem_size_mib: 256,
            smt: Some(false),
            track_dirty_pages: None,
            cpu_template: None,
        };
        let s = serde_json::to_value(&cfg).unwrap();
        assert_eq!(s["vcpu_count"], 2);
        assert_eq!(s["mem_size_mib"], 256);
        assert_eq!(s["smt"], false);
        assert!(s.get("track_dirty_pages").is_none());
        assert!(s.get("cpu_template").is_none());
    }

    #[test]
    fn boot_source_serializes_with_correct_keys() {
        let bs = BootSourceDto {
            kernel_image_path: "/opt/firecracker/vmlinux",
            boot_args: Some("console=ttyS0 reboot=k panic=1 quiet"),
        };
        let s = serde_json::to_value(&bs).unwrap();
        assert_eq!(s["kernel_image_path"], "/opt/firecracker/vmlinux");
        assert!(s["boot_args"].as_str().unwrap().contains("console=ttyS0"));
    }

    #[test]
    fn drive_dto_marks_rootfs_correctly() {
        let d = DriveDto {
            drive_id: "rootfs",
            path_on_host: "/opt/efe-nano/rootfs/ner_clinical.ext4",
            is_root_device: true,
            is_read_only: true,
        };
        let s = serde_json::to_value(&d).unwrap();
        assert_eq!(s["drive_id"], "rootfs");
        assert_eq!(s["is_root_device"], true);
        assert_eq!(s["is_read_only"], true);
    }

    #[test]
    fn vsock_dto_carries_cid_and_uds() {
        let v = VsockDto {
            guest_cid: 42,
            uds_path: "/var/run/efe-nano/vsock/vm-1.sock",
            vsock_id: "primary",
        };
        let s = serde_json::to_value(&v).unwrap();
        assert_eq!(s["guest_cid"], 42);
        assert_eq!(s["vsock_id"], "primary");
    }

    #[test]
    fn action_dto_serializes_instance_start() {
        let a = ActionDto {
            action_type: "InstanceStart",
        };
        let s = serde_json::to_value(&a).unwrap();
        assert_eq!(s["action_type"], "InstanceStart");
    }

    #[test]
    fn connect_to_nonexistent_socket_returns_connect_error() {
        let client = ApiClient::new("/dev/null/does-not-exist.sock", Duration::from_secs(1));
        let err = client.instance_start().unwrap_err();
        assert!(matches!(err, ApiError::Connect { .. }));
    }
}
