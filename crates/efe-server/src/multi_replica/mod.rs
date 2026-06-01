//! Sprint **D18** — Multi-réplica runtime support.
//!
//! Cf. `docs/bench/d18_multi_replica_bench_d12_architecture.md`.
//!
//! This module ships the 5 D18 building blocks for horizontally
//! scalable efe-server :
//!
//!   * [`idempotency_pg`] — PG-backed idempotency cache shared
//!     across N replicas (T-D18-2 mitigation).
//!   * [`ratelimit_pg`] — PG-backed leaky-bucket rate-limit
//!     shared across N replicas (T-D18-3 mitigation).
//!   * [`leader_election`] — `LeaderLock` trait + `MockLeaderLock`
//!     test impl + `K8sLeaseLock` production stub for K8s
//!     `coordination.k8s.io/v1 Lease` (T-D18-1, T-D18-4, T-D18-10
//!     mitigation).
//!   * [`shutdown_sequence`] — 3-phase graceful shutdown :
//!     drain RPC → audit flush → pg close (T-D18-8, T-D18-11
//!     mitigation).
//!   * [`health_grpc`] — `efe.health.v1.Health.Check` service
//!     supporting K8s liveness + readiness probes.

pub mod health_grpc;
pub mod idempotency_pg;
pub mod leader_election;
pub mod ratelimit_pg;
pub mod shutdown_sequence;

pub use health_grpc::{HealthService, HealthStatus};
pub use idempotency_pg::{
    IdempotencyOutcome, IdempotencyStore, MockIdempotencyStore, PgIdempotencyStore,
};
pub use leader_election::{K8sLeaseLock, LeaderLock, LeaderLockError, LeaseClaim, MockLeaderLock};
pub use ratelimit_pg::{
    MockRateLimitStore, PgRateLimitStore, RateLimitDecision, RateLimitError, RateLimitStore,
};
pub use shutdown_sequence::{ShutdownPhase, ShutdownSequence, ShutdownTimeouts};
