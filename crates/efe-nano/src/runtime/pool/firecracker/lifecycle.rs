//! **Sprint 8 — N11** : `Lifecycle` trait + impls.
//!
//! The [`Lifecycle`] trait abstracts "how to spawn / reap / talk to a VM" so the
//! pool logic is **identical** for tests (mock) and production (real Firecracker).
//!
//! Two impls :
//! - [`MockLifecycle`] — cross-platform, in-process.  Used by **every** unit
//!   test and by the Tier-2 integration tests.  Simulates spawn latency,
//!   inference work, and crashes via injectable closures.
//! - [`FirecrackerLifecycle`] — Linux + KVM only.  Spawns real Firecracker
//!   processes, opens vsock connections.  Sprint 8 days 11-15 deliverable.
//!   Compiled behind `#[cfg(target_os = "linux")]` ; macOS dev gets a
//!   compile-only stub so the IDE / cargo check pass.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;

use super::config::KindConfig;
use super::rpc::{VsockRpcRequest, VsockRpcResponse};
use super::types::{LifecycleHandle, SubPoolKey, VmId, VmState, WarmVm};

// ─────────────────────────────────────────────────────────────────────
//  Errors
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum LifecycleError {
    #[error("spawn failed : {0}")]
    SpawnFailed(String),

    #[error("vm boot timeout after {boot_budget_ms} ms")]
    BootTimeout { boot_budget_ms: u64 },

    #[error("vsock RPC failed : {0}")]
    RpcFailed(String),

    #[error("vsock timeout after {timeout_ms} ms")]
    RpcTimeout { timeout_ms: u64 },

    #[error("reap failed : {0}")]
    ReapFailed(String),

    #[error("not supported on this platform : {0}")]
    NotSupportedOnPlatform(String),

    #[error("internal : {0}")]
    Internal(String),
}

// ─────────────────────────────────────────────────────────────────────
//  Trait
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : abstraction over VM lifecycle + RPC.
///
/// The pool calls `spawn` to provision a new VM, `rpc` to send a request,
/// `reap` to destroy a VM, `is_alive` for health checks.  Implementations
/// are responsible for :
/// - Allocating the VsockCid (or mock-equivalent).
/// - Forking / mocking the Firecracker process.
/// - Health-pinging the daemon before declaring the VM warm.
/// - Cleaning up cgroups / sockets on reap.
#[async_trait]
pub trait Lifecycle: Send + Sync + 'static {
    /// Spawn a new VM for the given sub-pool key.  Returns a fully
    /// warm-ready [`WarmVm`] (i.e. the daemon has answered at least one
    /// successful health-ping).
    async fn spawn(
        &self,
        key: SubPoolKey,
        kind_config: KindConfig,
    ) -> Result<WarmVm, LifecycleError>;

    /// Send an RPC to a specific VM.  The handle inside `vm` knows
    /// where to dial.
    async fn rpc(
        &self,
        vm: &WarmVm,
        request: VsockRpcRequest,
        timeout: Duration,
    ) -> Result<VsockRpcResponse, LifecycleError>;

    /// Destroy a VM (SIGTERM → SIGKILL → cgroup cleanup).
    async fn reap(&self, vm: &WarmVm) -> Result<(), LifecycleError>;

    /// Cheap health probe : "is this VM still alive ?"  Used by the
    /// reaper background sweep.  Implementations should return quickly
    /// (≤ 100 ms) and prefer a vsock ping over a heavyweight check.
    async fn is_alive(&self, vm: &WarmVm) -> bool;

    /// Lifecycle identity string for tracing + metrics.
    fn name(&self) -> &'static str;
}

// ─────────────────────────────────────────────────────────────────────
//  MockLifecycle — cross-platform, in-process
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : in-process mock for tests.
///
/// Simulates :
/// - Configurable spawn latency (via `spawn_latency`).
/// - Configurable inference latency per call.
/// - Injectable failure modes : `fail_next_spawn`, `kill_after_n_rpcs`,
///   `respond_with_error`.
///
/// **Does NOT** actually fork processes, open vsock, allocate cgroups.
/// Everything lives in the test process's heap.
pub struct MockLifecycle {
    inner: Arc<MockState>,
}

struct MockState {
    spawn_latency: std::sync::Mutex<Duration>,
    rpc_latency: std::sync::Mutex<Duration>,
    /// Set to `Some(N)` to fail the next N `spawn` calls.
    fail_next_spawn: std::sync::Mutex<u32>,
    /// Set to `Some(N)` to kill each VM after it serves N RPCs.
    kill_after_n_rpcs: std::sync::Mutex<u32>,
    /// Per-VM RPC counter ; when it hits `kill_after_n_rpcs`, the VM dies.
    rpc_counts: dashmap::DashMap<VmId, u32>,
    /// Inference handler — defaults to echo.  Tests can install
    /// custom logic.
    handler: std::sync::Mutex<Arc<MockHandler>>,
    /// Set of VMs the test has marked "dead".  Used by `is_alive`.
    dead_vms: dashmap::DashSet<VmId>,
    /// Atomic tag counter for `LifecycleHandle::Mock`.
    next_tag: std::sync::atomic::AtomicU64,
    /// VmId allocator.
    vm_id_alloc: super::types::VmIdAllocator,
    /// CID allocator.
    cid_alloc: super::types::VsockCidAllocator,
    /// Total spawn count (telemetry for tests).
    spawn_count: std::sync::atomic::AtomicU64,
    /// Total reap count.
    reap_count: std::sync::atomic::AtomicU64,
}

/// **Sprint 8 — N11** : custom inference handler for tests.  Lets a test
/// inject deterministic, fake outputs without coupling to the real ONNX
/// runtime.
pub type MockHandler = dyn Fn(&VsockRpcRequest) -> VsockRpcResponse + Send + Sync;

impl MockLifecycle {
    pub fn new() -> Self {
        Self::with_handler(default_echo_handler())
    }

    pub fn with_handler(handler: Arc<MockHandler>) -> Self {
        Self {
            inner: Arc::new(MockState {
                spawn_latency: std::sync::Mutex::new(Duration::from_millis(10)),
                rpc_latency: std::sync::Mutex::new(Duration::from_millis(1)),
                fail_next_spawn: std::sync::Mutex::new(0),
                kill_after_n_rpcs: std::sync::Mutex::new(0),
                rpc_counts: dashmap::DashMap::new(),
                handler: std::sync::Mutex::new(handler),
                dead_vms: dashmap::DashSet::new(),
                next_tag: std::sync::atomic::AtomicU64::new(0),
                vm_id_alloc: super::types::VmIdAllocator::new(),
                cid_alloc: super::types::VsockCidAllocator::new(),
                spawn_count: std::sync::atomic::AtomicU64::new(0),
                reap_count: std::sync::atomic::AtomicU64::new(0),
            }),
        }
    }

    pub fn set_spawn_latency(&self, latency: Duration) {
        *self.inner.spawn_latency.lock().unwrap() = latency;
    }

    pub fn set_rpc_latency(&self, latency: Duration) {
        *self.inner.rpc_latency.lock().unwrap() = latency;
    }

    pub fn install_handler(&self, handler: Arc<MockHandler>) {
        *self.inner.handler.lock().unwrap() = handler;
    }

    /// Inject a failure into the next `n` spawn calls.
    pub fn fail_next_spawns(&self, n: u32) {
        *self.inner.fail_next_spawn.lock().unwrap() = n;
    }

    /// Schedule every VM to "die" after it serves `n` RPCs.  Useful for
    /// reaper tests.
    pub fn kill_each_vm_after(&self, n: u32) {
        *self.inner.kill_after_n_rpcs.lock().unwrap() = n;
    }

    /// Manually mark a VM as dead.  The next `is_alive(vm)` returns
    /// false ; the next `rpc(vm, ...)` errors with `RpcFailed`.
    pub fn kill_vm(&self, vm_id: VmId) {
        self.inner.dead_vms.insert(vm_id);
    }

    pub fn spawn_count(&self) -> u64 {
        self.inner
            .spawn_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn reap_count(&self) -> u64 {
        self.inner
            .reap_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Default for MockLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for MockLifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockLifecycle")
            .field("spawn_count", &self.spawn_count())
            .field("reap_count", &self.reap_count())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Lifecycle for MockLifecycle {
    async fn spawn(
        &self,
        key: SubPoolKey,
        _kind_config: KindConfig,
    ) -> Result<WarmVm, LifecycleError> {
        // Check injected failure.
        {
            let mut budget = self.inner.fail_next_spawn.lock().unwrap();
            if *budget > 0 {
                *budget -= 1;
                return Err(LifecycleError::SpawnFailed(
                    "mock injected spawn failure".into(),
                ));
            }
        }
        let latency = *self.inner.spawn_latency.lock().unwrap();
        tokio::time::sleep(latency).await;

        let id = self.inner.vm_id_alloc.allocate();
        let cid = self.inner.cid_alloc.allocate();
        let tag = self
            .inner
            .next_tag
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.inner
            .spawn_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let now = Instant::now();
        Ok(WarmVm {
            id,
            key,
            cid,
            firecracker_pid: 0,
            created_at: now,
            last_used_at: std::sync::Mutex::new(now),
            state: std::sync::Mutex::new(VmState::Warm),
            lifecycle_handle: LifecycleHandle::Mock { tag },
        })
    }

    async fn rpc(
        &self,
        vm: &WarmVm,
        request: VsockRpcRequest,
        timeout: Duration,
    ) -> Result<VsockRpcResponse, LifecycleError> {
        // Dead VM ?
        if self.inner.dead_vms.contains(&vm.id) {
            return Err(LifecycleError::RpcFailed(format!(
                "vm {} is dead (test-injected)",
                vm.id
            )));
        }

        // Simulate latency ; respect timeout.
        let latency = *self.inner.rpc_latency.lock().unwrap();
        if latency > timeout {
            tokio::time::sleep(timeout).await;
            return Err(LifecycleError::RpcTimeout {
                timeout_ms: timeout.as_millis() as u64,
            });
        }
        tokio::time::sleep(latency).await;

        // Increment per-VM RPC counter ; kill if past threshold.
        let kill_after = *self.inner.kill_after_n_rpcs.lock().unwrap();
        if kill_after > 0 {
            let mut count = self.inner.rpc_counts.entry(vm.id).or_insert(0);
            *count += 1;
            if *count >= kill_after {
                self.inner.dead_vms.insert(vm.id);
            }
        }

        let handler = Arc::clone(&self.inner.handler.lock().unwrap());
        let resp = (handler)(&request);
        vm.touch();
        Ok(resp)
    }

    async fn reap(&self, vm: &WarmVm) -> Result<(), LifecycleError> {
        self.inner
            .reap_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.inner.dead_vms.insert(vm.id);
        self.inner.rpc_counts.remove(&vm.id);
        Ok(())
    }

    async fn is_alive(&self, vm: &WarmVm) -> bool {
        !self.inner.dead_vms.contains(&vm.id)
    }

    fn name(&self) -> &'static str {
        "mock"
    }
}

/// **Sprint 8 audit** : public test helper for re-installing the
/// default echo handler.  Used by regression tests that install a
/// failure handler then need to revert to success behavior.
#[doc(hidden)]
pub fn default_echo_handler_for_test() -> Arc<MockHandler> {
    default_echo_handler()
}

/// Default echo-style handler : returns a success response with
/// `output = { "echoed": <input> }`.
fn default_echo_handler() -> Arc<MockHandler> {
    use super::rpc::{InferResult, RpcMethod, RpcPayload, RpcResult};
    use std::collections::HashMap;
    Arc::new(move |req: &VsockRpcRequest| -> VsockRpcResponse {
        match &req.method {
            RpcMethod::Infer { params } => VsockRpcResponse {
                schema_version: super::rpc::SCHEMA_VERSION,
                request_id: req.request_id,
                payload: RpcPayload::Ok {
                    result: RpcResult::Infer(InferResult {
                        output: serde_json::json!({ "echoed": params.input }),
                        model_invocation_micros: 1000,
                        model_memory_high_water_mb: 64,
                        guest_metadata: HashMap::new(),
                    }),
                },
            },
            RpcMethod::Health => VsockRpcResponse {
                schema_version: super::rpc::SCHEMA_VERSION,
                request_id: req.request_id,
                payload: RpcPayload::Ok {
                    result: RpcResult::Health(super::rpc::HealthResult {
                        status: "ready".into(),
                        uptime_secs: 0,
                        model_id: "mock".into(),
                        model_version: "0.0.0".into(),
                    }),
                },
            },
            RpcMethod::Shutdown { .. } => VsockRpcResponse {
                schema_version: super::rpc::SCHEMA_VERSION,
                request_id: req.request_id,
                payload: RpcPayload::Ok {
                    result: RpcResult::Shutdown { acknowledged: true },
                },
            },
        }
    })
}

// ─────────────────────────────────────────────────────────────────────
//  FirecrackerLifecycle — Linux only
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : real Firecracker integration.
///
/// **Status** : Sprint 8 days 11-15 deliverable.  Sprint 8 days 1-10
/// ship the trait surface + MockLifecycle ; this struct is a Linux-only
/// stub that returns `NotSupportedOnPlatform` on macOS and a documented
/// TODO on Linux.
///
/// **Sprint 8 — N11 days 11-15** : real Firecracker integration.
///
/// Wires :
/// - [`super::api_client::ApiClient`] for VM config over Unix socket.
/// - [`super::cgroup::CgroupController`] for v2 limits.
/// - [`super::vsock_client::UnixSocketBackend`] for inference RPC.
/// - `tokio::process::Command` for the Firecracker child process.
///
/// Per-VM lifecycle :
/// 1. Allocate `VmId` + `VsockCid`.
/// 2. Build paths : `api_socket_dir/vm-{id}.sock`, `api_socket_dir/vsock/vm-{id}.sock`.
/// 3. `cgroup.create_vm_cgroup(id, kind)` ; capture path.
/// 4. `tokio::process::Command::new(binary).arg("--api-sock", api_socket).spawn()`.
/// 5. Wait for API socket (poll up to `boot_budget / 4`).
/// 6. `ApiClient::put_machine_config` → `put_boot_source` → `put_rootfs`
///    → `put_vsock` → `instance_start`.
/// 7. Move PID into cgroup via `CgroupController::add_pid`.
/// 8. Wait for vsock UDS (poll up to `boot_budget / 4`).
/// 9. Send `Health` ping ; require Ok within remaining budget.
/// 10. Return `WarmVm`.
#[cfg(target_os = "linux")]
pub struct FirecrackerLifecycle {
    config: super::config::FirecrackerPoolConfig,
    vm_id_alloc: super::types::VmIdAllocator,
    cid_alloc: super::types::VsockCidAllocator,
    cgroup: super::cgroup::CgroupController,
    /// Holds the spawned children so they're not orphaned at process
    /// exit.  Cleaned up on `reap`.
    children: tokio::sync::Mutex<
        std::collections::HashMap<super::types::VmId, tokio::process::Child>,
    >,
}

#[cfg(target_os = "linux")]
impl std::fmt::Debug for FirecrackerLifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FirecrackerLifecycle")
            .field("binary_path", &self.config.binary_path)
            .field("kernel_image", &self.config.kernel_image)
            .finish_non_exhaustive()
    }
}

#[cfg(target_os = "linux")]
impl FirecrackerLifecycle {
    /// Construct.  Verifies KVM availability + binary paths.  Requires
    /// the parent cgroup to exist + have the right controllers
    /// (`+cpu +memory +io` in `subtree_control`) ; use
    /// [`CgroupController::ensure_parent`](super::cgroup::CgroupController::ensure_parent)
    /// from the deployment script if needed.
    pub fn new(config: super::config::FirecrackerPoolConfig) -> Result<Self, LifecycleError> {
        if !std::path::Path::new("/dev/kvm").exists() {
            return Err(LifecycleError::NotSupportedOnPlatform(
                "/dev/kvm not present — KVM kernel module not loaded?".into(),
            ));
        }
        if !config.binary_path.exists() {
            return Err(LifecycleError::SpawnFailed(format!(
                "firecracker binary not found at {}",
                config.binary_path.display()
            )));
        }
        if !config.kernel_image.exists() {
            return Err(LifecycleError::SpawnFailed(format!(
                "kernel image not found at {}",
                config.kernel_image.display()
            )));
        }
        let cgroup = super::cgroup::CgroupController::new(&config.cgroup_root)
            .map_err(|e| LifecycleError::SpawnFailed(format!("cgroup init : {e}")))?;

        // Ensure the api socket + vsock directories exist.
        std::fs::create_dir_all(&config.api_socket_dir).map_err(|e| {
            LifecycleError::SpawnFailed(format!(
                "mkdir -p {} : {e}",
                config.api_socket_dir.display()
            ))
        })?;
        let vsock_dir = config.api_socket_dir.join("vsock");
        std::fs::create_dir_all(&vsock_dir).map_err(|e| {
            LifecycleError::SpawnFailed(format!(
                "mkdir -p {} : {e}",
                vsock_dir.display()
            ))
        })?;

        Ok(Self {
            config,
            vm_id_alloc: super::types::VmIdAllocator::new(),
            cid_alloc: super::types::VsockCidAllocator::new(),
            cgroup,
            children: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    fn api_socket_for(&self, id: super::types::VmId) -> std::path::PathBuf {
        self.config
            .api_socket_dir
            .join(format!("vm-{}.sock", id.raw()))
    }

    fn vsock_uds_for(&self, id: super::types::VmId) -> std::path::PathBuf {
        self.config
            .api_socket_dir
            .join("vsock")
            .join(format!("vm-{}.sock", id.raw()))
    }
}

#[cfg(target_os = "linux")]
#[async_trait]
impl Lifecycle for FirecrackerLifecycle {
    async fn spawn(
        &self,
        key: SubPoolKey,
        kind_config: KindConfig,
    ) -> Result<WarmVm, LifecycleError> {
        let started = std::time::Instant::now();

        // 1. Allocate IDs.
        let vm_id = self.vm_id_alloc.allocate();
        let cid = self.cid_alloc.allocate();
        let api_socket = self.api_socket_for(vm_id);
        let vsock_uds = self.vsock_uds_for(vm_id);

        // Pre-cleanup : remove any stale socket files from a previous run.
        let _ = std::fs::remove_file(&api_socket);
        let _ = std::fs::remove_file(&vsock_uds);

        // 2. Create cgroup BEFORE forking so we can move the PID in
        //    as soon as the process starts.
        let cgroup_path =
            self.cgroup
                .create_vm_cgroup(vm_id, &kind_config)
                .map_err(|e| {
                    LifecycleError::SpawnFailed(format!(
                        "cgroup setup for {vm_id} : {e}"
                    ))
                })?;

        // 3. Fork the Firecracker child.
        let mut command = tokio::process::Command::new(&self.config.binary_path);
        command
            .arg("--api-sock")
            .arg(&api_socket)
            .arg("--id")
            .arg(format!("efe-nano-vm-{}", vm_id.raw()))
            // Suppress firecracker's stdout/stderr (or pipe them — we
            // pipe so we can log on crash).
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let child = command.spawn().map_err(|e| {
            // Best-effort cgroup cleanup on spawn failure.
            let _ = self.cgroup.remove_vm_cgroup(vm_id);
            LifecycleError::SpawnFailed(format!("fork firecracker : {e}"))
        })?;
        let pid = child.id().unwrap_or(0);

        // 4. Move PID into cgroup.  Best-effort : if PID is 0 (rare race
        //    where child already exited), the move will fail and we'll
        //    detect via API connect.
        if pid != 0 {
            if let Err(e) = super::cgroup::CgroupController::add_pid(&cgroup_path, pid) {
                tracing::warn!(
                    target: "efe_nano::firecracker",
                    %vm_id, %pid, error = ?e,
                    "could not move firecracker pid into cgroup ; isolation reduced"
                );
            }
        }

        // 5. Wait for the API socket to appear.  Firecracker creates it
        //    a few ms after fork.
        let api_wait_budget = self.config.boot_budget / 4;
        if let Err(_) = wait_for_uds_sync(&api_socket, api_wait_budget).await {
            self.cleanup_on_failure(vm_id, child).await;
            return Err(LifecycleError::BootTimeout {
                boot_budget_ms: api_wait_budget.as_millis() as u64,
            });
        }

        // 6. Configure the VM via the API.
        let api_timeout = self.config.boot_budget / 8;
        let api = super::api_client::ApiClient::new(&api_socket, api_timeout);

        if let Err(e) = api.put_machine_config(&kind_config) {
            self.cleanup_on_failure(vm_id, child).await;
            return Err(LifecycleError::SpawnFailed(format!(
                "put_machine_config : {e}"
            )));
        }
        if let Err(e) = api.put_boot_source(
            &self.config.kernel_image,
            Some(
                "console=ttyS0 reboot=k panic=1 quiet \
                 efe.daemon_port=5000",
            ),
        ) {
            self.cleanup_on_failure(vm_id, child).await;
            return Err(LifecycleError::SpawnFailed(format!(
                "put_boot_source : {e}"
            )));
        }
        if let Err(e) = api.put_rootfs(&kind_config.rootfs_path) {
            self.cleanup_on_failure(vm_id, child).await;
            return Err(LifecycleError::SpawnFailed(format!(
                "put_rootfs : {e}"
            )));
        }
        if let Err(e) = api.put_vsock(cid, &vsock_uds) {
            self.cleanup_on_failure(vm_id, child).await;
            return Err(LifecycleError::SpawnFailed(format!(
                "put_vsock : {e}"
            )));
        }
        if let Err(e) = api.instance_start() {
            self.cleanup_on_failure(vm_id, child).await;
            return Err(LifecycleError::SpawnFailed(format!(
                "instance_start : {e}"
            )));
        }

        // 7. Wait for the guest vsock UDS to appear.
        let remaining = self
            .config
            .boot_budget
            .checked_sub(started.elapsed())
            .unwrap_or(Duration::from_millis(50));
        let vsock_wait = remaining / 2;
        if let Err(_) = super::vsock_client::wait_for_uds(&vsock_uds, vsock_wait).await {
            self.cleanup_on_failure(vm_id, child).await;
            return Err(LifecycleError::BootTimeout {
                boot_budget_ms: self.config.boot_budget.as_millis() as u64,
            });
        }

        // 8. Health-ping to confirm the daemon is ready.
        let backend = super::vsock_client::UnixSocketBackend::new(&vsock_uds, 5000);
        let health_req = VsockRpcRequest::health(uuid::Uuid::new_v4());
        let health_resp = backend
            .call(health_req, remaining.saturating_sub(vsock_wait))
            .await
            .map_err(|e| {
                LifecycleError::BootTimeout {
                    boot_budget_ms: self.config.boot_budget.as_millis() as u64,
                }
                .into_health_failure(e)
            });
        let health_resp = match health_resp {
            Ok(r) => r,
            Err(e) => {
                self.cleanup_on_failure(vm_id, child).await;
                return Err(e);
            }
        };
        if !matches!(
            health_resp.payload,
            super::rpc::RpcPayload::Ok {
                result: super::rpc::RpcResult::Health(_)
            }
        ) {
            self.cleanup_on_failure(vm_id, child).await;
            return Err(LifecycleError::SpawnFailed(
                "guest health ping returned non-Health Ok".into(),
            ));
        }

        // 9. Store the child handle so reap can kill it later.
        self.children.lock().await.insert(vm_id, child);

        // 10. Build the WarmVm.
        let now = std::time::Instant::now();
        Ok(WarmVm {
            id: vm_id,
            key,
            cid,
            firecracker_pid: pid,
            created_at: now,
            last_used_at: std::sync::Mutex::new(now),
            state: std::sync::Mutex::new(VmState::Warm),
            lifecycle_handle: LifecycleHandle::Firecracker {
                api_socket,
                pid,
            },
        })
    }

    async fn rpc(
        &self,
        vm: &WarmVm,
        request: VsockRpcRequest,
        timeout: Duration,
    ) -> Result<VsockRpcResponse, LifecycleError> {
        let vsock_uds = self.vsock_uds_for(vm.id);
        let backend = super::vsock_client::UnixSocketBackend::new(&vsock_uds, 5000);
        backend.call(request, timeout).await.map_err(|e| match e {
            super::vsock_client::VsockError::Timeout(d) => LifecycleError::RpcTimeout {
                timeout_ms: d.as_millis() as u64,
            },
            super::vsock_client::VsockError::Connect { reason, .. } => {
                LifecycleError::RpcFailed(format!("vsock connect : {reason}"))
            }
            super::vsock_client::VsockError::Io(reason) => {
                LifecycleError::RpcFailed(format!("vsock I/O : {reason}"))
            }
            super::vsock_client::VsockError::Frame(e) => {
                LifecycleError::RpcFailed(format!("frame error : {e}"))
            }
        })
    }

    async fn reap(&self, vm: &WarmVm) -> Result<(), LifecycleError> {
        // Pull the child handle out so we can wait on it.
        let mut child_opt = self.children.lock().await.remove(&vm.id);

        // 1. Try graceful shutdown via API (SendCtrlAltDel).
        let api_socket = self.api_socket_for(vm.id);
        if api_socket.exists() {
            let api = super::api_client::ApiClient::new(&api_socket, Duration::from_millis(200));
            // Best-effort ; ignore errors (the VM may already be dying).
            let _ = api.graceful_shutdown();
        }

        // 2. SIGTERM, wait 2s, SIGKILL.
        if let Some(child) = child_opt.as_mut() {
            if let Some(pid) = child.id() {
                signal::term(pid);
                // Wait up to 2 seconds for graceful exit.
                let wait_result = tokio::time::timeout(
                    Duration::from_secs(2),
                    child.wait(),
                )
                .await;
                if wait_result.is_err() {
                    // Timeout : escalate to SIGKILL.
                    signal::kill(pid);
                    let _ = child.wait().await;
                }
            }
        }

        // 3. Cleanup cgroup (rmdir).  May fail if processes still in it ;
        //    log and continue (PID 0 case during error recovery).
        if let Err(e) = self.cgroup.remove_vm_cgroup(vm.id) {
            tracing::warn!(
                target: "efe_nano::firecracker",
                vm_id = %vm.id,
                error = ?e,
                "cgroup rmdir failed during reap ; will retry next sweep"
            );
        }

        // 4. Cleanup socket files.
        let _ = std::fs::remove_file(self.api_socket_for(vm.id));
        let _ = std::fs::remove_file(self.vsock_uds_for(vm.id));

        Ok(())
    }

    async fn is_alive(&self, vm: &WarmVm) -> bool {
        // Two checks :
        // 1. Is the Firecracker process still running ?
        // 2. Does it respond to a health ping within 100 ms ?

        if vm.firecracker_pid != 0 {
            let proc_status = std::path::Path::new("/proc")
                .join(vm.firecracker_pid.to_string())
                .join("status");
            if !proc_status.exists() {
                return false;
            }
        }

        // Health ping.
        let vsock_uds = self.vsock_uds_for(vm.id);
        if !vsock_uds.exists() {
            return false;
        }
        let backend = super::vsock_client::UnixSocketBackend::new(&vsock_uds, 5000);
        let req = VsockRpcRequest::health(uuid::Uuid::new_v4());
        backend
            .call(req, Duration::from_millis(100))
            .await
            .is_ok()
    }

    fn name(&self) -> &'static str {
        "firecracker"
    }
}

#[cfg(target_os = "linux")]
impl FirecrackerLifecycle {
    /// Helper : full cleanup of a partially-provisioned VM on the
    /// boot-failure path.  Kills the child, removes the cgroup, removes
    /// the sockets.  Best-effort throughout (no error propagation —
    /// we're already returning a failure to the caller).
    async fn cleanup_on_failure(&self, vm_id: super::types::VmId, mut child: tokio::process::Child) {
        if let Some(pid) = child.id() {
            signal::kill(pid);
        }
        let _ = child.wait().await;
        let _ = self.cgroup.remove_vm_cgroup(vm_id);
        let _ = std::fs::remove_file(self.api_socket_for(vm_id));
        let _ = std::fs::remove_file(self.vsock_uds_for(vm_id));
    }
}

/// **Sprint 8 audit fix** : safe wrapper around `libc::kill` so the
/// crate-level `#![deny(unsafe_code)]` can stay strict.  This module
/// is the only place in `efe-nano` that uses `unsafe`, and the call
/// is sound : we pass a PID that the kernel cannot have re-used
/// because we still hold a `tokio::process::Child` handle for it
/// (Linux guarantees PID stability while the parent holds an unwaited
/// child).
#[cfg(target_os = "linux")]
mod signal {
    #[allow(unsafe_code)]
    pub(super) fn term(pid: u32) {
        // SAFETY : pid is owned by us (parent of the child process)
        // until we wait on it ; the kernel won't re-use it.  SIGTERM
        // never has UB.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }

    #[allow(unsafe_code)]
    pub(super) fn kill(pid: u32) {
        // SAFETY : same as `term` above.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
}

/// **Sprint 8 audit fix** : non-blocking wait-for-UDS.
///
/// Pre-fix this called `std::os::unix::net::UnixStream::connect` which
/// is a **blocking syscall** in the tokio runtime — under load it can
/// starve the tokio worker thread and cause cascading timeouts on
/// other tasks.  Post-fix uses `tokio::net::UnixStream::connect`
/// (truly async) so the runtime stays responsive.
#[cfg(target_os = "linux")]
async fn wait_for_uds_sync(
    path: &std::path::Path,
    total_timeout: Duration,
) -> Result<(), LifecycleError> {
    let start = std::time::Instant::now();
    while start.elapsed() < total_timeout {
        // tokio::fs::metadata is async ; avoids the std::path::exists
        // blocking syscall.
        if tokio::fs::metadata(path).await.is_ok() {
            // File exists ; verify we can connect.  Use the tokio
            // async UnixStream (NOT std::os::unix::net::UnixStream
            // which would block the runtime).
            if tokio::net::UnixStream::connect(path).await.is_ok() {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err(LifecycleError::BootTimeout {
        boot_budget_ms: total_timeout.as_millis() as u64,
    })
}

#[cfg(target_os = "linux")]
impl LifecycleError {
    fn into_health_failure(self, src: super::vsock_client::VsockError) -> LifecycleError {
        LifecycleError::SpawnFailed(format!("guest health ping failed : {src} (boot path : {self})"))
    }
}

/// Compile-only stub on non-Linux.  Macros prevent the test suite from
/// instantiating it ; `cargo check` on macOS dev passes.
#[cfg(not(target_os = "linux"))]
#[derive(Debug)]
pub struct FirecrackerLifecycle {
    _phantom: std::marker::PhantomData<()>,
}

#[cfg(not(target_os = "linux"))]
impl FirecrackerLifecycle {
    pub fn new(
        _config: super::config::FirecrackerPoolConfig,
    ) -> Result<Self, LifecycleError> {
        Err(LifecycleError::NotSupportedOnPlatform(
            "FirecrackerLifecycle requires Linux + KVM".into(),
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::TenantClass;
    use uuid::Uuid;

    fn config() -> KindConfig {
        KindConfig::testing_mode_b("test_model", "1.0.0")
    }

    fn key() -> SubPoolKey {
        SubPoolKey::mode_b("test_model", "1.0.0", TenantClass::Tier1)
    }

    // ─────────────────────────────────────────────────────────────────
    //  MockLifecycle — spawn + reap
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn mock_spawn_returns_warm_vm() {
        let lc = MockLifecycle::new();
        let vm = lc.spawn(key(), config()).await.unwrap();
        assert!(matches!(*vm.state.lock().unwrap(), VmState::Warm));
        assert_eq!(lc.spawn_count(), 1);
    }

    #[tokio::test]
    async fn mock_spawn_allocates_distinct_ids_and_cids() {
        let lc = MockLifecycle::new();
        let vm1 = lc.spawn(key(), config()).await.unwrap();
        let vm2 = lc.spawn(key(), config()).await.unwrap();
        assert_ne!(vm1.id, vm2.id);
        assert_ne!(vm1.cid, vm2.cid);
    }

    #[tokio::test]
    async fn mock_spawn_respects_injected_failure() {
        let lc = MockLifecycle::new();
        lc.fail_next_spawns(2);
        assert!(lc.spawn(key(), config()).await.is_err());
        assert!(lc.spawn(key(), config()).await.is_err());
        // Third spawn succeeds.
        assert!(lc.spawn(key(), config()).await.is_ok());
    }

    #[tokio::test]
    async fn mock_reap_increments_counter_and_kills_vm() {
        let lc = MockLifecycle::new();
        let vm = lc.spawn(key(), config()).await.unwrap();
        assert!(lc.is_alive(&vm).await);
        lc.reap(&vm).await.unwrap();
        assert_eq!(lc.reap_count(), 1);
        assert!(!lc.is_alive(&vm).await);
    }

    // ─────────────────────────────────────────────────────────────────
    //  MockLifecycle — RPC
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn mock_rpc_returns_echo_by_default() {
        let lc = MockLifecycle::new();
        let vm = lc.spawn(key(), config()).await.unwrap();
        let req = VsockRpcRequest::infer(
            Uuid::new_v4(),
            serde_json::json!({"text": "hello"}),
            "test_model",
            "1.0.0",
            Uuid::new_v4(),
            i64::MAX,
        );
        let resp = lc.rpc(&vm, req, Duration::from_secs(1)).await.unwrap();
        match resp.payload {
            super::super::rpc::RpcPayload::Ok {
                result: super::super::rpc::RpcResult::Infer(r),
            } => {
                assert_eq!(r.output["echoed"]["text"], "hello");
            }
            other => panic!("expected Infer Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mock_rpc_to_dead_vm_returns_error() {
        let lc = MockLifecycle::new();
        let vm = lc.spawn(key(), config()).await.unwrap();
        lc.kill_vm(vm.id);
        let req = VsockRpcRequest::health(Uuid::new_v4());
        let err = lc.rpc(&vm, req, Duration::from_secs(1)).await.unwrap_err();
        assert!(err.to_string().contains("dead"));
    }

    #[tokio::test]
    async fn mock_rpc_respects_timeout() {
        let lc = MockLifecycle::new();
        lc.set_rpc_latency(Duration::from_millis(50));
        let vm = lc.spawn(key(), config()).await.unwrap();
        let req = VsockRpcRequest::health(Uuid::new_v4());
        let start = Instant::now();
        let err = lc.rpc(&vm, req, Duration::from_millis(10)).await.unwrap_err();
        let elapsed = start.elapsed();
        assert!(matches!(err, LifecycleError::RpcTimeout { .. }));
        assert!(
            elapsed >= Duration::from_millis(10) && elapsed <= Duration::from_millis(40),
            "elapsed must be ~ timeout, got {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn mock_kill_after_n_rpcs_actually_kills() {
        let lc = MockLifecycle::new();
        lc.kill_each_vm_after(3);
        let vm = lc.spawn(key(), config()).await.unwrap();
        for _ in 0..3 {
            let req = VsockRpcRequest::health(Uuid::new_v4());
            lc.rpc(&vm, req, Duration::from_secs(1)).await.unwrap();
        }
        assert!(!lc.is_alive(&vm).await);
    }

    #[tokio::test]
    async fn mock_custom_handler_intercepts() {
        let lc = MockLifecycle::new();
        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);
        lc.install_handler(Arc::new(move |req: &VsockRpcRequest| {
            counter_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            VsockRpcResponse::ok(
                req.request_id,
                super::super::rpc::RpcResult::Health(super::super::rpc::HealthResult {
                    status: "custom".into(),
                    uptime_secs: 42,
                    model_id: "x".into(),
                    model_version: "y".into(),
                }),
            )
        }));
        let vm = lc.spawn(key(), config()).await.unwrap();
        let req = VsockRpcRequest::health(Uuid::new_v4());
        let resp = lc.rpc(&vm, req, Duration::from_secs(1)).await.unwrap();
        assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), 1);
        match resp.payload {
            super::super::rpc::RpcPayload::Ok {
                result: super::super::rpc::RpcResult::Health(h),
            } => assert_eq!(h.status, "custom"),
            _ => panic!("expected custom Health"),
        }
    }

    #[tokio::test]
    async fn mock_spawn_latency_is_observable() {
        let lc = MockLifecycle::new();
        lc.set_spawn_latency(Duration::from_millis(20));
        let start = Instant::now();
        let _ = lc.spawn(key(), config()).await.unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(20) && elapsed < Duration::from_millis(100),
            "spawn latency ~ 20ms, got {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn mock_lifecycle_name() {
        let lc = MockLifecycle::new();
        assert_eq!(lc.name(), "mock");
    }

    #[tokio::test]
    async fn mock_concurrent_spawns_yield_distinct_vms() {
        let lc = Arc::new(MockLifecycle::new());
        let mut handles = Vec::new();
        for _ in 0..16 {
            let lc = Arc::clone(&lc);
            handles.push(tokio::spawn(async move {
                lc.spawn(key(), config()).await.unwrap().id
            }));
        }
        let mut ids = Vec::new();
        for h in handles {
            ids.push(h.await.unwrap());
        }
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 16);
        assert_eq!(lc.spawn_count(), 16);
    }

    // ─────────────────────────────────────────────────────────────────
    //  FirecrackerLifecycle — platform gating
    // ─────────────────────────────────────────────────────────────────

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn firecracker_lifecycle_macos_returns_unsupported() {
        let cfg = super::super::config::FirecrackerPoolConfig::for_testing();
        let err = FirecrackerLifecycle::new(cfg).unwrap_err();
        assert!(matches!(err, LifecycleError::NotSupportedOnPlatform(_)));
    }

    // Linux unit tests can run without KVM as long as they don't actually
    // spawn ; they verify the prerequisite checks and stub error paths.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn firecracker_lifecycle_linux_stub_returns_unimplemented() {
        let cfg = super::super::config::FirecrackerPoolConfig::for_testing();
        // Construction MAY fail if /dev/kvm is absent — fine for CI.
        if let Ok(lc) = FirecrackerLifecycle::new(cfg) {
            let err = lc.spawn(key(), config()).await.unwrap_err();
            assert!(matches!(err, LifecycleError::NotSupportedOnPlatform(_)));
        }
    }
}
