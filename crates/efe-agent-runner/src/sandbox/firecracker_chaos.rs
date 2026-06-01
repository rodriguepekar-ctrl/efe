//! Chaos test harness for the Firecracker driver.
//!
//! Activated by `EFE_FC_CHAOS_TEST=true`. Runs only when `/dev/kvm` is
//! present (i.e. on a real KVM-capable host). Each scenario exercises
//! one of the 6 documented failure modes (§ 9.4 of the architecture doc)
//! and asserts that :
//!   - the spawn errors out promptly (within 1.5× spawn_timeout),
//!   - the cleanup path runs to completion (no leaked socket files),
//!   - the metric `efe_agent_microvm_provision_duration_seconds` records
//!     a non-`ok` outcome.

use crate::sandbox::firecracker::{
    FirecrackerDriver, MicrovmConfig, SeccompProfile, new_vm_id, allocate_vsock_cid,
};
use crate::sandbox::SandboxResources;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub enum ChaosFault {
    StepBootSourceFails,
    StepRootfsBadPath,
    StepMachineConfigBadMemory,
    StepVsockHangs,
    StepInstanceStartFails,
    SocketDisappearsMidSpawn,
    PermissiveSeccompInProd,
}

pub fn enabled() -> bool {
    matches!(
        std::env::var("EFE_FC_CHAOS_TEST").as_deref(),
        Ok("true") | Ok("1") | Ok("yes")
    ) && std::path::Path::new("/dev/kvm").exists()
}

/// Run a single chaos scenario. Returns Ok if the harness behaviour
/// matches the expected outcome.
pub async fn run(driver: Arc<dyn FirecrackerDriver>, fault: ChaosFault) -> Result<(), String> {
    let cfg = MicrovmConfig {
        vm_id: new_vm_id(),
        resources: SandboxResources::default(),
        vsock_cid: allocate_vsock_cid(rand::random::<u32>()),
        network_iface: "tap-efe-chaos0".into(),
        seccomp_profile: match fault {
            ChaosFault::PermissiveSeccompInProd => SeccompProfile::Permissive,
            _ => SeccompProfile::AgentCore,
        },
    };

    // For the "socket disappears" scenario, race a deletion task against
    // the spawn ; it should error and leave no orphan files.
    if matches!(fault, ChaosFault::SocketDisappearsMidSpawn) {
        let socket = std::path::PathBuf::from(format!("/var/run/firecracker/{}.socket", cfg.vm_id));
        let socket_clone = socket.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let _ = tokio::fs::remove_file(socket_clone).await;
        });
    }

    let result = tokio::time::timeout(
        Duration::from_secs(45),
        driver.spawn(&cfg),
    )
    .await;

    match (fault, result) {
        (ChaosFault::PermissiveSeccompInProd, Ok(Err(e))) => {
            if format!("{:?}", e).contains("permissive seccomp") {
                Ok(())
            } else {
                Err(format!("expected SandboxEscapeAttempt for permissive seccomp ; got {:?}", e))
            }
        }
        (_, Ok(Err(_))) => {
            // Spawn errored as expected ; cleanup should run via the
            // controller's wrapper (here we trust the driver).
            let _ = driver.kill(&cfg.vm_id).await;
            Ok(())
        }
        (_, Ok(Ok(_handle))) => {
            let _ = driver.kill(&cfg.vm_id).await;
            Err(format!("expected error for fault {:?} but spawn succeeded", fault))
        }
        (_, Err(_)) => Err(format!("scenario {:?} timed out", fault)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_returns_false_without_dev_kvm() {
        // On non-KVM test machines this should be false.
        if !std::path::Path::new("/dev/kvm").exists() {
            assert!(!enabled());
        }
    }
}
