// SPDX-License-Identifier: UNLICENSED
//! **Sprint 8 — N15** : Fairness scheduling subsystem.
//!
//! See `docs/architecture/EFE_M50_N15_FAIRNESS_SCHEDULING_ARCHITECTURE.md`
//! for the full design — incident patterns (P1-P6), commitments
//! (G1-G12), invariants (I1-I20), failure modes (F1-F16), ADRs, plan,
//! annexes.
//!
//! # Module surface
//!
//! | Module          | Wave | Purpose                                                         |
//! |-----------------|------|-----------------------------------------------------------------|
//! | [`policy_lookup`] | 2  | `TenantPolicyLookup` trait + in-memory impl.                    |
//! | [`drr`]         |  3   | `DrrScheduler` + `DrrPermit` RAII for slot-counted resources.   |
//! | [`token_bucket`] | 4   | `TokenBucket` per (tenant, provider) for rate-limited resources.|
//! | [`tracker`]     |  5   | `InflightTracker` — per-tenant atomic counters.                 |
//! | [`admission`]   |  6   | `AdmissionGate` + `AdmissionTicket` RAII (front door).          |
//! | [`metrics`]     |  7   | 12 Prometheus-compatible counters / gauges / histograms.        |
//!
//! # Wiring
//!
//! Every public type is `Send + Sync + 'static` so the gate can be
//! shared via `Arc<AdmissionGate>` across every Dispatcher / executor
//! task without contention beyond the per-tenant atomic / Notify
//! surfaces.
//!
//! # Sprint scope
//!
//! - **Sprint 8 (this ship)** : DRR + token bucket + tracker +
//!   admission gate + metrics, integrated into the Dispatcher front
//!   door (Wave 8).
//! - **Sprint 9 (deferred)** : WFQ for queue-bounded resources,
//!   cross-tenant work-conserving borrow with priority inversion,
//!   per-tenant policy overrides in Postgres.
//!
//! # Backward compatibility
//!
//! The Dispatcher accepts `Option<Arc<AdmissionGate>>` so all existing
//! 1 136 tests continue to pass without an admission step.  Production
//! wiring installs the gate at construction.

pub mod admission;
pub mod drr;
pub mod metrics;
pub mod policy_lookup;
pub mod token_bucket;
pub mod tracker;

pub use admission::{
    AdmissionGate, AdmissionGateBuilder, AdmissionTicket, DEFAULT_ACQUIRE_TIMEOUT,
    DEFAULT_RETRY_AFTER_MS,
};
pub use drr::{DrrPermit, DrrScheduler};
pub use metrics::{
    Histogram, HistogramSnapshot, RegistryKind, SchedulerKind, SchedulerMetrics,
    SchedulerMetricsSnapshot, KIND_COUNT, REGISTRY_KIND_COUNT, SCHEDULER_KIND_COUNT, TIER_COUNT,
};
pub use policy_lookup::{InMemoryTenantPolicyLookup, TenantPolicyLookup};
pub use token_bucket::{TokenBucket, TokenBucketRegistry};
pub use tracker::{InflightSnapshot, InflightTracker};
