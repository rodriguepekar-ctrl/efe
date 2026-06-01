//! `efe-smoke` — 9-check merge-gate suite (PR #23).
//!
//! See `docs/EFE_A1_PHASE2_PR23_ARCHITECTURE.md` (STABILITY_CONTRACT v1.5 § 23).

#![warn(rust_2018_idioms, clippy::all)]
#![allow(clippy::module_name_repetitions, missing_docs)]

pub mod build;
pub mod checks;
pub mod cli;
pub mod cluster_glue;
pub mod error;
pub mod gate;
pub mod loadgen_client;
pub mod metrics_scrape;
pub mod orchestrator;
pub mod otel_collector;
pub mod redact;
pub mod result;

pub use crate::checks::{Check, CheckContext, CheckResult, CheckVerdict, SmokeCheckId};
pub use crate::error::{ExitCode, SmokeError, SmokeResult};
pub use crate::orchestrator::{SmokeConfig, SmokeOrchestrator};

/// 9 frozen smoke check IDs in stable order (STABILITY_CONTRACT v1.5 § 23.1).
pub const SMOKE_CHECK_IDS: &[&str] = &[
    "S1_boot",
    "S2_l1_swarm",
    "S3_l2_meta_swarm",
    "S4_mtls_rotation",
    "S5_reload_sighup",
    "S6_metrics",
    "S7_otel",
    "S8_restart",
    "S9_shutdown",
];

/// Per-check wall-clock budget in seconds (STABILITY_CONTRACT v1.5 § 23.2).
pub mod budgets {
    use std::time::Duration;

    pub const S1_BOOT: Duration = Duration::from_secs(30);
    pub const S2_L1_SWARM: Duration = Duration::from_secs(15);
    pub const S3_L2_META_SWARM: Duration = Duration::from_secs(20);
    pub const S4_MTLS_ROTATION: Duration = Duration::from_secs(25);
    pub const S5_RELOAD_SIGHUP: Duration = Duration::from_secs(20);
    pub const S6_METRICS: Duration = Duration::from_secs(10);
    pub const S7_OTEL: Duration = Duration::from_secs(15);
    pub const S8_RESTART: Duration = Duration::from_secs(30);
    pub const S9_SHUTDOWN: Duration = Duration::from_secs(15);

    /// Total per-check sum (excluding cluster boot ; AD3).
    pub const TOTAL: Duration = Duration::from_secs(
        S1_BOOT.as_secs()
            + S2_L1_SWARM.as_secs()
            + S3_L2_META_SWARM.as_secs()
            + S4_MTLS_ROTATION.as_secs()
            + S5_RELOAD_SIGHUP.as_secs()
            + S6_METRICS.as_secs()
            + S7_OTEL.as_secs()
            + S8_RESTART.as_secs()
            + S9_SHUTDOWN.as_secs(),
    );

    /// Global hard cap (240 s — AD3).
    pub const GLOBAL_HARD_CAP: Duration = Duration::from_secs(240);
}

/// 14 STABLE metric families validated by S6 (PR #17 STABILITY_CONTRACT
/// v1.0 § 4 — frozen).  Locked by `tests/stability_contract_v15.rs`.
pub const STABLE_METRIC_FAMILIES: &[&str] = &[
    "efe_l2_request_total",
    "efe_l2_request_duration_seconds_bucket",
    "efe_swarm_recovery_event_total",
    "efe_swarm_recovery_seconds",
    "efe_replication_lag_bucket",
    "efe_replication_lag",
    "efe_controller_participants_total",
    "efe_controller_uptime_seconds",
    "efe_participant_join_total",
    "efe_participant_uptime_seconds",
    "efe_l2_active_connections",
    "efe_l2_tls_handshakes_total",
    "efe_l2_tls_handshakes_failed_total",
    "efe_swarm_size_current",
];

/// Schema version of the smoke result JSON (STABILITY_CONTRACT v1.5 § 23.5).
pub const SMOKE_RESULT_SCHEMA_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nine_smoke_check_ids() {
        assert_eq!(SMOKE_CHECK_IDS.len(), 9);
    }

    #[test]
    fn fourteen_stable_metric_families() {
        // PR #17 STABILITY_CONTRACT v1.0 § 4 — locked.
        assert_eq!(STABLE_METRIC_FAMILIES.len(), 14);
    }

    #[test]
    fn total_budget_is_180_seconds() {
        assert_eq!(budgets::TOTAL.as_secs(), 180);
    }

    #[test]
    fn global_cap_is_240_seconds() {
        assert_eq!(budgets::GLOBAL_HARD_CAP.as_secs(), 240);
    }
}
