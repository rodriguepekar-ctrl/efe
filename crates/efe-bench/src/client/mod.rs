//! Sprint D18 + D29 — gRPC client wrappers.
//!
//! Workloads are written against the [`BenchClient`] trait so they
//! stay protocol-agnostic for unit tests. Two impls ship :
//!
//! * [`MockBenchClient`] — instant in-process responses for harness
//!   self-tests + smoke runs that need zero network. **Default
//!   pre-D29 ; minoritary post-D29 (operators must opt in via
//!   `--allow-mock`).**
//! * [`TonicBenchClient`] — real gRPC over `tonic::transport::Channel`.
//!   Default post-D29 ; gated behind the `tonic` cargo feature so
//!   build hosts without `protoc` keep compiling.
//!
//! ## ClientKind
//!
//! Every impl returns a [`ClientKind`] tag which the report
//! generator records in the JSON `client_kind` field (schema v2).
//! Downstream tooling can split bench reports by client kind to
//! distinguish "harness self-test" runs from "real-stack" runs.

use std::time::Duration;

use async_trait::async_trait;

use crate::error::BenchError;
use crate::workload::WorkloadOp;

pub mod mock;

#[cfg(feature = "tonic")]
pub mod tonic_client;

pub use mock::MockBenchClient;

#[cfg(feature = "tonic")]
pub use tonic_client::{TonicBenchClient, TonicConfig};

/// **Sprint D29 (Φ-D)** — symbolic tag for the client backend.
/// Recorded in the JSON `BenchReport` (schema v2) so cross-client
/// comparisons are unambiguous (`mock` runs are clearly identifiable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    /// In-process stub ; instant latency, no network.
    Mock,
    /// Real Tonic-backed gRPC client.
    Tonic,
}

impl ClientKind {
    /// Stable string used in JSON / metric labels.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Tonic => "tonic",
        }
    }
}

/// Outcome of a single gRPC call.
#[derive(Debug, Clone)]
pub struct CallOutcome {
    /// Operation that was attempted.
    pub op: WorkloadOp,
    /// Wall-clock latency.
    pub latency: Duration,
    /// `true` iff the call succeeded.
    pub ok: bool,
    /// Error label for failed calls (e.g. "Unavailable", "DeadlineExceeded").
    pub error_label: Option<String>,
}

/// A bench client capable of issuing the 7 workload op types
/// (6 D18 + 1 D38 Mode-3).
///
/// **Sprint D29 (Φ-D)** — the trait gained a [`Self::kind`] method
/// so the report can record which backend executed each run.
///
/// **Audit fix M6-A1 (Φ-F)** — the `call` method gained an explicit
/// `worker_id: u64` parameter. Pre-audit the Tonic client used a
/// private atomic counter (`call_idx`) to derive the
/// `x-efe-gpu-shard-id` metadata header. That counter rotated
/// across all workers on every call, so two requests from the
/// same `worker_id` could land on different shards depending on
/// scheduling. The bench-side per-shard metrics, however, were
/// keyed on `worker_id % gpu_shards` (per-worker stable
/// assignment), creating a **misalignment** between the metrics
/// the report exposed and the shard each request actually hit.
/// Post-audit the runner passes `worker_id` to every call ; both
/// the metric bucket and the propagated metadata key on the same
/// value, so the per-shard report reflects the real GPU dispatch.
#[async_trait]
pub trait BenchClient: Send + Sync + std::fmt::Debug {
    /// Execute one op and return the timing + outcome.
    ///
    /// `tenant` is the symbolic tenant id used for multi-tenant
    /// fairness checks ; `worker_id` is the runner-side per-worker
    /// counter (stable for the lifetime of the bench run). The
    /// Tonic client uses `worker_id` to derive the gRPC metadata
    /// `x-efe-gpu-shard-id: <worker_id % gpu_shards>` (D31). Mock
    /// ignores it (no network = no metadata).
    async fn call(
        &self,
        op: WorkloadOp,
        tenant: &str,
        worker_id: u64,
    ) -> Result<CallOutcome, BenchError>;

    /// Symbolic backend tag for the report. Default `Mock` so any
    /// future stub impl that forgets to override it lands in the
    /// safe / opt-in bucket.
    fn kind(&self) -> ClientKind {
        ClientKind::Mock
    }

    /// **Sprint D31 (Φ-D)** — number of GPU shards this client
    /// orchestrates. Used by the report generator to populate the
    /// top-level `gpu_shards` field. Default `None` (single GPU
    /// or N/A) ; the Tonic client overrides it when configured
    /// with `--gpu-shards N`.
    fn gpu_shards(&self) -> Option<u32> {
        None
    }
}
