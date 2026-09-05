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
// `api_client`, `cgroup` et `vsock_client` ne vivent que sous Linux, mais la garde
// est ÉCRITE DANS CHAQUE FICHIER (`#![cfg(target_os = "linux")]`), pas ici. Elle
// l'était aux DEUX endroits, et clippy sur Linux l'a dit : `duplicated attribute`.
// Nous gardons celle du fichier, jamais celle-ci : une garde posée sur `mod` empêche
// le fichier d'être seulement LU sur macOS ; posée dans le fichier, il est encore
// analysé (ses éléments seuls disparaissent). C'est le sens qui nous manquait —
// tout ce code n'avait jamais été compilé nulle part hors d'un runner Linux.
pub mod api_client;
/// **Sprint 8 — N11 days 11-15** : cgroup v2 writer (Linux-only).
/// Enforces per-VM CPU / memory / IO limits.
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
