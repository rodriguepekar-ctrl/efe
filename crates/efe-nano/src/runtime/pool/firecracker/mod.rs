//! **Sprint 8 — N11** : Firecracker VM pool.
//!
//! See `docs/architecture/EFE_M50_N11_FIRECRACKER_POOL_ARCHITECTURE.md`
//! for the design document.
//!
//! # Module layout
//!
//! - [`types`] : core data types (`VmId`, `VsockCid`, `TenantClass`,
//!   `SubPoolKey`, `VmState`, `WarmVm`, `ExecutionOutcome`).
//! - [`config`] : [`FirecrackerPoolConfig`] + [`KindConfig`].
//! - [`rpc`] : vsock RPC framing (magic byte + length-prefix) + message
//!   types (`VsockRpcRequest`, `VsockRpcResponse`, `InferParams`, ...).
//! - [`lifecycle`] : [`Lifecycle`] trait + `MockLifecycle` (tests) +
//!   `FirecrackerLifecycle` (Linux + KVM production).
//! - [`metrics`] : 14 Prometheus counters / gauges / histograms.
//! - [`kind_pool`] : [`KindPool`] sub-pool data structure.
//! - [`reaper`] : background sweep + provision task.
//! - [`pool`] : [`FirecrackerVmPool`] — the top-level `VmPool` impl.
//!
//! # Quick start (tests / dev)
//!
//! ```ignore
//! use efe_nano::runtime::pool::firecracker::FirecrackerVmPool;
//! use efe_nano::runtime::pool::{VmKind, VmPool, VmRequest};
//! use std::time::Duration;
//!
//! # async fn example() {
//! let pool = FirecrackerVmPool::for_testing();
//! let mut lease = pool
//!     .acquire(VmKind::ModelInference, Duration::from_secs(1))
//!     .await
//!     .unwrap();
//! let req = VmRequest::mode_b(
//!     "ner_clinical",
//!     "1.0.0",
//!     serde_json::json!({"text": "patient took 200mg aspirin"}),
//!     5000,
//!     "ner_clinical",
//!     "1.0.0",
//! );
//! let resp = lease.execute(req).await.unwrap();
//! println!("{}", resp.output);
//! # }
//! ```

/// **Sprint 8 — N11 days 11-15** : Firecracker control-plane HTTP
/// client (Linux-only).  Provisions VMs via the Firecracker JSON API
/// over Unix-domain socket.
#[cfg(target_os = "linux")]
pub mod api_client;
/// **Sprint 8 — N11 days 11-15** : cgroup v2 writer (Linux-only).
/// Enforces per-VM CPU / memory / IO limits.
#[cfg(target_os = "linux")]
pub mod cgroup;
pub mod config;
pub mod kind_pool;
pub mod lifecycle;
pub mod metrics;
pub mod pool;
pub mod reaper;
pub mod rpc;
pub mod types;
/// **Sprint 8 — N11 days 12-13** : vsock RPC client (Linux-only).
/// Communicates with the guest inference daemon over Firecracker's
/// vsock-over-UDS proxy.
#[cfg(target_os = "linux")]
pub mod vsock_client;

pub use config::{ConfigError, FirecrackerPoolConfig, KindConfig, SubPoolBudget};
pub use kind_pool::KindPool;
pub use lifecycle::{FirecrackerLifecycle, Lifecycle, LifecycleError, MockHandler, MockLifecycle};
pub use metrics::{Counter, FirecrackerMetrics, Gauge, Histogram, SubPoolMetrics};
pub use pool::FirecrackerVmPool;
pub use reaper::Reaper;
pub use rpc::{
    decode_frame, encode_frame, encode_frame_bytes, parse_frame_header, FrameError, HealthResult,
    InferParams, InferResult, RpcError, RpcErrorCode, RpcMethod, RpcPayload, RpcResult,
    VsockRpcRequest, VsockRpcResponse, FRAME_HEADER_LEN, FRAME_MAGIC, MAX_RPC_BYTES,
    SCHEMA_VERSION,
};
pub use types::{
    ExecutionOutcome, LifecycleHandle, SubPoolKey, TenantClass, VmId, VmIdAllocator, VmState,
    VsockCid, VsockCidAllocator, WarmVm,
};
