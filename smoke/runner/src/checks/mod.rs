//! PR #23 § 7 — `Check` trait + 9 check kernels.
//!
//! Each of S1–S9 is a separate module that implements [`Check`] :
//!
//!   * [`s1_boot`]            — controller + 3 participants ready
//!   * [`s2_l1_swarm`]        — 5 canonical L1 RPCs respond
//!   * [`s3_l2_meta_swarm`]   — L2 RPCs respond
//!   * [`s4_mtls_rotation`]   — cert rotation mid-traffic, 0 dropped
//!   * [`s5_reload_sighup`]   — SIGHUP reload preserves in-flight
//!   * [`s6_metrics`]         — 14 STABLE Prom families present
//!   * [`s7_otel`]            — OTel chain has ≥ 2 spans
//!   * [`s8_restart`]         — pod restart within drain deadline
//!   * [`s9_shutdown`]        — SIGTERM exits 0 within 10 s

pub mod s1_boot;
pub mod s2_l1_swarm;
pub mod s3_l2_meta_swarm;
pub mod s4_mtls_rotation;
pub mod s5_reload_sighup;
pub mod s6_metrics;
pub mod s7_otel;
pub mod s8_restart;
pub mod s9_shutdown;

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::SmokeResult;

/// 9 frozen check IDs (STABILITY_CONTRACT v1.5 § 23.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SmokeCheckId {
    S1,
    S2,
    S3,
    S4,
    S5,
    S6,
    S7,
    S8,
    S9,
}

impl SmokeCheckId {
    #[must_use]
    pub const fn all() -> [Self; 9] {
        [Self::S1, Self::S2, Self::S3, Self::S4, Self::S5, Self::S6, Self::S7, Self::S8, Self::S9]
    }

    /// Frozen string form (e.g. `"S1_boot"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::S1 => "S1_boot",
            Self::S2 => "S2_l1_swarm",
            Self::S3 => "S3_l2_meta_swarm",
            Self::S4 => "S4_mtls_rotation",
            Self::S5 => "S5_reload_sighup",
            Self::S6 => "S6_metrics",
            Self::S7 => "S7_otel",
            Self::S8 => "S8_restart",
            Self::S9 => "S9_shutdown",
        }
    }

    /// Short form (e.g. `"S1"`).
    #[must_use]
    pub const fn short(self) -> &'static str {
        match self {
            Self::S1 => "S1",
            Self::S2 => "S2",
            Self::S3 => "S3",
            Self::S4 => "S4",
            Self::S5 => "S5",
            Self::S6 => "S6",
            Self::S7 => "S7",
            Self::S8 => "S8",
            Self::S9 => "S9",
        }
    }

    /// Parse from either short (`S1`) or long (`S1_boot`) form.
    pub fn parse(s: &str) -> Option<Self> {
        Self::all()
            .into_iter()
            .find(|id| id.short().eq_ignore_ascii_case(s) || id.as_str().eq_ignore_ascii_case(s))
    }

    /// Per-check budget (STABILITY_CONTRACT v1.5 § 23.2).
    #[must_use]
    pub const fn budget(self) -> Duration {
        match self {
            Self::S1 => crate::budgets::S1_BOOT,
            Self::S2 => crate::budgets::S2_L1_SWARM,
            Self::S3 => crate::budgets::S3_L2_META_SWARM,
            Self::S4 => crate::budgets::S4_MTLS_ROTATION,
            Self::S5 => crate::budgets::S5_RELOAD_SIGHUP,
            Self::S6 => crate::budgets::S6_METRICS,
            Self::S7 => crate::budgets::S7_OTEL,
            Self::S8 => crate::budgets::S8_RESTART,
            Self::S9 => crate::budgets::S9_SHUTDOWN,
        }
    }
}

/// Binary verdict (AD25 — no warn / indeterminate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckVerdict {
    Pass,
    Fail,
}

impl CheckVerdict {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
        }
    }
}

/// Per-check structured result (AD16).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub id: SmokeCheckId,
    pub verdict: CheckVerdict,
    pub duration_s: f64,
    pub budget_s: f64,
    /// Free-form `serde_json::Value` for per-check diagnostics ; not part
    /// of the v1.5 frozen contract (forward-compatible — AD40).
    pub details: serde_json::Value,
    /// Operator-readable failure summary (empty if `verdict == Pass`).
    pub error_summary: String,
}

impl CheckResult {
    #[must_use]
    pub fn pass(id: SmokeCheckId, duration: Duration, details: serde_json::Value) -> Self {
        Self {
            id,
            verdict: CheckVerdict::Pass,
            duration_s: duration.as_secs_f64(),
            budget_s: id.budget().as_secs_f64(),
            details,
            error_summary: String::new(),
        }
    }

    #[must_use]
    pub fn fail(
        id: SmokeCheckId,
        duration: Duration,
        error_summary: String,
        details: serde_json::Value,
    ) -> Self {
        Self {
            id,
            verdict: CheckVerdict::Fail,
            duration_s: duration.as_secs_f64(),
            budget_s: id.budget().as_secs_f64(),
            details,
            error_summary,
        }
    }
}

/// Execution context handed to each [`Check::run`] invocation.
pub struct CheckContext {
    pub kubeconfig: PathBuf,
    pub chart_version: String,
    pub namespace: String,
    /// Per-check timeout (operator override of [`SmokeCheckId::budget`]).
    pub timeout: Duration,
    /// If true, the check uses MOCK in-memory fixtures and never touches
    /// the cluster (used by unit tests + `--mock` CLI flag for dev).
    pub mock: bool,
    pub redact_logs: bool,
}

/// The contract every smoke check implements (AD16).
#[async_trait]
pub trait Check: Send + Sync {
    fn id(&self) -> SmokeCheckId;
    #[must_use]
    fn budget(&self) -> Duration {
        self.id().budget()
    }
    async fn run(&self, ctx: &CheckContext) -> SmokeResult<CheckResult>;
}

/// Build all 9 check kernels in stable order.
#[must_use]
pub fn all_checks() -> Vec<Box<dyn Check>> {
    vec![
        Box::new(s1_boot::S1Boot),
        Box::new(s2_l1_swarm::S2L1Swarm),
        Box::new(s3_l2_meta_swarm::S3L2MetaSwarm),
        Box::new(s4_mtls_rotation::S4MtlsRotation),
        Box::new(s5_reload_sighup::S5ReloadSighup),
        Box::new(s6_metrics::S6Metrics),
        Box::new(s7_otel::S7Otel),
        Box::new(s8_restart::S8Restart),
        Box::new(s9_shutdown::S9Shutdown),
    ]
}

/// Dispatch to the kernel for a given check id.  Used by integration
/// tests + the in-cluster runner when invoking a single check.
#[must_use]
pub fn dispatch_for(id: SmokeCheckId) -> Box<dyn Check> {
    match id {
        SmokeCheckId::S1 => Box::new(s1_boot::S1Boot),
        SmokeCheckId::S2 => Box::new(s2_l1_swarm::S2L1Swarm),
        SmokeCheckId::S3 => Box::new(s3_l2_meta_swarm::S3L2MetaSwarm),
        SmokeCheckId::S4 => Box::new(s4_mtls_rotation::S4MtlsRotation),
        SmokeCheckId::S5 => Box::new(s5_reload_sighup::S5ReloadSighup),
        SmokeCheckId::S6 => Box::new(s6_metrics::S6Metrics),
        SmokeCheckId::S7 => Box::new(s7_otel::S7Otel),
        SmokeCheckId::S8 => Box::new(s8_restart::S8Restart),
        SmokeCheckId::S9 => Box::new(s9_shutdown::S9Shutdown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nine_check_ids_in_stable_order() {
        let ids = SmokeCheckId::all();
        assert_eq!(ids.len(), 9);
        assert_eq!(ids[0].short(), "S1");
        assert_eq!(ids[8].short(), "S9");
    }

    #[test]
    fn check_id_strings_frozen() {
        // STABILITY_CONTRACT v1.5 § 23.1.
        assert_eq!(SmokeCheckId::S1.as_str(), "S1_boot");
        assert_eq!(SmokeCheckId::S6.as_str(), "S6_metrics");
        assert_eq!(SmokeCheckId::S9.as_str(), "S9_shutdown");
    }

    #[test]
    fn parse_accepts_short_and_long() {
        assert_eq!(SmokeCheckId::parse("S1"), Some(SmokeCheckId::S1));
        assert_eq!(SmokeCheckId::parse("S1_boot"), Some(SmokeCheckId::S1));
        assert_eq!(SmokeCheckId::parse("s9_shutdown"), Some(SmokeCheckId::S9));
        assert!(SmokeCheckId::parse("S10").is_none());
    }

    #[test]
    fn budgets_sum_180s() {
        let sum: u64 = SmokeCheckId::all().iter().map(|c| c.budget().as_secs()).sum();
        assert_eq!(sum, 180);
    }

    #[test]
    fn all_checks_returns_9() {
        assert_eq!(all_checks().len(), 9);
    }

    #[test]
    fn check_result_pass_constructor() {
        let r = CheckResult::pass(SmokeCheckId::S1, Duration::from_secs(5), serde_json::json!({}));
        assert_eq!(r.verdict, CheckVerdict::Pass);
        assert_eq!(r.duration_s, 5.0);
        assert_eq!(r.budget_s, 30.0);
    }

    #[test]
    fn check_result_fail_carries_summary() {
        let r = CheckResult::fail(
            SmokeCheckId::S4,
            Duration::from_secs(22),
            "2 dropped requests".into(),
            serde_json::json!({"dropped": 2}),
        );
        assert_eq!(r.verdict, CheckVerdict::Fail);
        assert!(r.error_summary.contains("dropped"));
    }
}
