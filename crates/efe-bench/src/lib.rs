//! Sprint **D18** — Bench harness for D12 catalog operations +
//! `Execute` throughput.
//!
//! Sprint **D29 (Φ-D)** + **D31 (Φ-D)** + **D38 (Φ-F)** —
//! M6 extension. The crate now ships :
//!
//!   * **8 workload patterns** — `read-heavy`, `write-heavy`,
//!     `balanced`, `burst`, `multi-tenant`, `soak` (D18) plus
//!     `mode3-k5` and `mode3-k10` (D38).
//!   * **2 client backends** — [`MockBenchClient`] (legacy D18,
//!     instant in-process responses) and [`TonicBenchClient`]
//!     (D29, real gRPC over `tonic::transport::Channel` ;
//!     gated on the `tonic` feature).
//!   * **Multi-GPU sharding** — `--gpu-shards N` propagates the
//!     `x-efe-gpu-shard-id` metadata header so a multi-GPU
//!     cluster can be benched as a fleet (D31).
//!   * **Mode-3 dispatch** — `WorkloadOp::Mode3Execute { k_swarm }`
//!     issues `efe.v1.mode3.Mode3Service.Execute` calls (D38) ;
//!     the harness validates `--k-swarm K` against the soft cap
//!     of 5 + the hard cap of 10 (S-12).
//!   * **JSON report v2** — adds `client_kind`, `gpu_shards`,
//!     `k_swarm`, `per_shard` fields ; v1 readers ignore them and
//!     v2 readers fall back gracefully on v1 files.
//!   * **3 derived SLOs** — `D31-GPU-1` (per-shard fairness),
//!     `C-3` (Mode-3 p99 ≤ 3 × Mode-1 p99), `C-9` (parallel
//!     speedup ≥ 2.5×). Cf. `report::{evaluate_c3, evaluate_c9}`.
//!
//! # Production safety
//!
//! `efe-bench` REFUSES to run against `prod` / `production`
//! target environments (T-D18-12 mitigation). The CLI argument
//! `--target-env` is checked at startup ; the value `prod` (any
//! case) returns [`error::BenchError::RefusedProdTarget`].
//!
//! # SLO mapping
//!
//! Each bench run produces a [`report::BenchReport`] structure
//! with per-SLO `pass/fail` flags matching architecture § 3.3 :
//!
//!   * `D18-bench-1` — `Execute` p99 ≤ 200 ms at 1000 RPS
//!   * `D18-bench-2` — `register_catalog` p95 ≤ 50 ms at 100 RPS
//!   * `D18-bench-3` — `resolve_catalog` p99 ≤ 20 ms at 5000 RPS
//!   * `D18-bench-4` — `list_catalogs` p95 ≤ 100 ms (any cardinality)
//!   * `D18-bench-5` — `register_class` + `register_sentence` ≥ 200 ops/s
//!   * `D18-bench-6` — multi-replica linear speedup ≥ 0.85 between 1↔3
//!   * `D18-bench-7` — audit chain HMAC sign p99 ≤ 5 ms
//!   * `D31-GPU-1`   — Jain fairness across GPU shards ≥ 0.95 (when N>1)
//!   * `C-3`         — Mode-3 p99 ≤ 3 × Mode-1 p99 (post-bench compare)
//!   * `C-9`         — Mode-3 k=5 throughput ≥ 2.5 × k=1 throughput
//!
//! # Feature flags
//!
//! * `tonic` (default OFF) — pulls in `tonic` + `prost` and
//!   compiles the `TonicBenchClient` plus the proto stubs from
//!   `protos/efe/v1/`. Without this feature the crate ships
//!   `MockBenchClient` only and the build does not require
//!   `protoc` (Mac/Windows CI runners stay green).

#![warn(missing_docs)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]

pub mod client;
pub mod error;
pub mod gen;
pub mod metrics;
pub mod report;
pub mod runner;
pub mod workload;

pub use client::{BenchClient, ClientKind, MockBenchClient};

#[cfg(feature = "tonic")]
pub use client::{TonicBenchClient, TonicConfig};

pub use error::{BenchError, TonicError};
pub use gen::{gen_dataset, BenchTenant, Dataset, DatasetSpec};
pub use metrics::{LatencyHistogram, RpsCounter};
pub use report::{
    canonical_slos, evaluate_c3, evaluate_c9, generate_report, generate_report_with_client,
    jain_index, BenchReport, OpReport, ShardReport, SloPassFail, SloTarget,
    C3_MODE3_LATENCY_RATIO, C9_MIN_SPEEDUP, D31_GPU_FAIRNESS_FLOOR,
};
// **Audit fix M6-A2 (Φ-F)** — re-export `OpStats` + `canonical_slos`
// + `SloTarget` so external consumers parsing a `RunnerOutcome`
// (e.g. an offline aggregator script written in Rust) can type
// their fields. Pre-audit these were `pub` inside their parent
// modules but invisible from the crate root, forcing consumers
// to import via the long `efe_bench::runner::OpStats` path.
pub use runner::{OpStats, Runner, RunnerConfig, RunnerOutcome, ShardStats};
pub use workload::{
    validate_gpu_shards, validate_k_swarm, validate_target_env, Workload, WorkloadOp,
    K_SWARM_HARD_CAP, K_SWARM_SOFT_CAP,
};

/// Schema version of the bench report JSON. Pinned so a future
/// reader can refuse unknown versions.
///
/// **Sprint D29 (Φ-D)** — bumped from `1.0.0` to `2.0.0` when the
/// schema gained `client_kind`, `gpu_shards`, `k_swarm`, and
/// `per_shard` fields. v1 readers ignore the new fields ; v2
/// readers fall back gracefully on v1 files (all new fields are
/// `Option<T>` or default to a safe value).
pub const REPORT_VERSION: &str = "2.0.0";
