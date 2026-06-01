//! Sandbox subsystem : Firecracker microVM driver + pool + network.
//!
//! **CRITICAL** : this is the security boundary. Every escape attempt
//! → metric `efe_agent_sandbox_escape_attempts_total` → PagerDuty P0.
//!
//! ## Layers
//!
//! 1. [`firecracker`] : low-level driver (spawn / kill / status).
//! 2. [`pool`]        : per-tenant pool of warm microVMs.
//! 3. [`network`]     : NetworkPolicy enforcement (deny-all + whitelist).
//! 4. [`image`]       : image whitelist + cosign verification.
//!
//! Every microVM has :
//! - Read-only rootfs (oci-mount style).
//! - Memory: 512 MiB (default), bumpable via profile.
//! - vCPU: 1 (default), bumpable via profile.
//! - No host filesystem access (vsock for control + state only).
//! - egress restricted to LLM router pod + mcp-server pod (via NetPol).

pub mod firecracker;
pub mod firecracker_chaos;        // M26
pub mod firecracker_real;         // M26
pub mod image;
pub mod lease_guard;              // M26
pub mod network;
pub mod pool;

pub use firecracker::{FirecrackerDriver, FirecrackerHandle, MicrovmConfig};
pub use firecracker_real::FirecrackerRealDriver;          // M26
pub use image::{verify_image_whitelist, ImageWhitelist};
pub use lease_guard::MicrovmLease;                        // M26
pub use network::{apply_network_policy, NetworkPolicySpec};
pub use pool::{MicrovmPool, MicrovmSlot, PoolConfig, SlotState};

use serde::{Deserialize, Serialize};

/// Resource specification for a single microVM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxResources {
    pub vcpus: u8,
    pub memory_mib: u32,
    pub rootfs_image: String,         // OCI ref, must be cosign-signed
    pub kernel_image: String,         // bzImage path on the host
    pub initramfs: Option<String>,    // optional ramdisk
}

impl Default for SandboxResources {
    fn default() -> Self {
        Self {
            vcpus: 1,
            memory_mib: 512,
            rootfs_image: "ghcr.io/efe-ai/agent-core:2.0.0".into(),
            kernel_image: "/opt/efe/firecracker/vmlinux".into(),
            initramfs: None,
        }
    }
}

/// Sandbox-level events surfaced to the controller (audited as
/// `sandbox_escape_attempt_detected` when malicious).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxEvent {
    /// microVM successfully spawned.
    Spawned { vm_id: String, took_ms: u64 },
    /// microVM exited normally (clean shutdown from agent-core).
    ExitedClean { vm_id: String, exit_code: i32 },
    /// microVM crashed or was killed for a reason other than escape.
    Crashed { vm_id: String, reason: String },
    /// **SECURITY-CRITICAL** : a syscall, ptrace, network, or filesystem
    /// access outside the allowed envelope was detected.
    EscapeAttempt {
        vm_id: String,
        kind: EscapeKind,
        details_internal: String,  // NEVER returned to caller — logged + audited only.
    },
}

/// Categories of escape attempts (label dimension on the security metric).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscapeKind {
    /// seccomp/audit denied a syscall not in the agent-core whitelist.
    SyscallDenied,
    /// process tried to attach to another via ptrace.
    Ptrace,
    /// outbound traffic blocked by NetworkPolicy (other than allowed pods).
    NetworkEgressDenied,
    /// agent tried to swap the rootfs image post-spawn.
    ImageSwap,
    /// agent tried to read/write outside its read-only mount.
    FilesystemBreakout,
    /// kvm-level escape (extremely unlikely, but tracked).
    KvmEscape,
}

impl EscapeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SyscallDenied => "syscall_denied",
            Self::Ptrace => "ptrace",
            Self::NetworkEgressDenied => "network_egress_denied",
            Self::ImageSwap => "image_swap",
            Self::FilesystemBreakout => "filesystem_breakout",
            Self::KvmEscape => "kvm_escape",
        }
    }
}
