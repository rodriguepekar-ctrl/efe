// SPDX-License-Identifier: UNLICENSED
//! # `efe-region` — M30 region awareness + cross-region health +
//! manual failover orchestration + quarterly DR drill harness.
//!
//! This crate is consumed by `efe-server` (REST handlers under
//! `v1/region/`) and by the standalone `efe-dr-drill` binary that
//! Compliance runs every quarter to produce SOC 2 evidence.
//!
//! ## Scope
//!
//! - **Current-region awareness** : reads `EFE_REGION`,
//!   `EFE_REGION_ROLE`, `EFE_REGION_PEER_BASE_URLS` from environment.
//! - **Cross-region health probing** : periodic supervisor + on-demand
//!   probe ; Prometheus gauge `efe_region_peer_up{region=…}`.
//! - **Failover orchestrator** : manual (operator-driven), validates
//!   preconditions, hands off to the target region's REST endpoint.
//! - **DR drill harness** : scripted failover-then-failback with
//!   structured `DrillReport` for compliance archives.
//!
//! ## Out of scope (deferred)
//!
//! - Automatic failover (M31 — requires multi-region consensus).
//! - Live cutover of NATS JetStream (M31).
//! - Tenant-pinning of read replicas (M31 — see ADR-M30-009).
//!
//! ## Architectural invariants
//!
//! 1. **No automatic role flips.** Every flip is operator-triggered
//!    AND logged with `operator` + `reason` in the HMAC audit chain.
//! 2. **No silent state in this crate.** All state lives in the
//!    `efe-server` SQL store ; this crate is stateless.

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]
#![deny(unsafe_code)]

pub mod current;
pub mod dr_drill;
pub mod error;
pub mod failover;
pub mod health;
pub mod metrics;
pub mod replication_monitor;

pub use current::{CurrentRegion, RegionRole};
pub use dr_drill::{DrDrillHarness, DrillConfig, DrillReport, DrillStep, DrillVerdict};
pub use error::{RegionError, Result};
pub use failover::{
    FailoverOrchestrator, FailoverRequest, FailoverResult, NoopQuiescer, Quiescer,
    MAX_LAG_SECS_FOR_FAILOVER,
};
pub use health::{run_periodic_supervisor, HealthProber, PeerHealth, ProbeConfig};
pub use metrics::set_replication_lag_secs;
pub use replication_monitor::{run_lag_monitor, LagFetcher, SlotLag};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reexports_construct() {
        let _err: RegionError = RegionError::Internal { reason: "x".into() };
    }
}
