//! PR #23 § 7.2 — **S2 L1 SwarmService** : 5 canonical L1 RPCs respond
//! < 500 ms p99 with `error_rate == 0`.
//!
//! The 5 RPCs (in order) :
//!   1. `CreateSwarm`
//!   2. `JoinSwarm`
//!   3. `ListSwarms`
//!   4. `LeaveSwarm`
//!   5. `DeleteSwarm`
//!
//! Issued via PR #20 loadgen library (in-process — AD20).  Seed 42 (AD23).

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::json;
use tracing::info;

use crate::checks::{Check, CheckContext, CheckResult, SmokeCheckId};
use crate::error::SmokeResult;

pub struct S2L1Swarm;

/// Frozen p99 budget (AD13 + § 7.2 — 500 ms).
const P99_BUDGET_MS: f64 = 500.0;

/// Frozen RPC list (STABILITY_CONTRACT v1.5 § 23.1 — semantic anchor).
pub const L1_RPCS: [&str; 5] = [
    "CreateSwarm",
    "JoinSwarm",
    "ListSwarms",
    "LeaveSwarm",
    "DeleteSwarm",
];

#[async_trait]
impl Check for S2L1Swarm {
    fn id(&self) -> SmokeCheckId {
        SmokeCheckId::S2
    }

    async fn run(&self, ctx: &CheckContext) -> SmokeResult<CheckResult> {
        let started = Instant::now();
        info!(check = "S2", mock = ctx.mock, "S2 L1 starting");

        if ctx.mock {
            return Ok(CheckResult::pass(
                self.id(),
                started.elapsed(),
                json!({
                    "mock": true,
                    "rpcs_succeeded": 5,
                    "p99_ms": 87.2,
                    "error_rate": 0.0,
                }),
            ));
        }

        // ── Real path : invoke loadgen library against the L1 surface.
        let summary = crate::loadgen_client::run_l1_smoke(ctx).await?;

        let p99_ms = summary.p99_ms;
        let succeeded = summary.rpcs_succeeded;
        let error_rate = summary.error_rate;

        if succeeded < 5 || error_rate > 0.0 {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!("L1 RPCs : succeeded={succeeded}/5, error_rate={error_rate}"),
                json!({"rpcs_succeeded": succeeded, "p99_ms": p99_ms, "error_rate": error_rate}),
            ));
        }
        if p99_ms > P99_BUDGET_MS {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!("p99 {p99_ms:.1}ms > budget {P99_BUDGET_MS}ms"),
                json!({"rpcs_succeeded": 5, "p99_ms": p99_ms, "error_rate": error_rate}),
            ));
        }

        Ok(CheckResult::pass(
            self.id(),
            started.elapsed(),
            json!({"rpcs_succeeded": 5, "p99_ms": p99_ms, "error_rate": error_rate}),
        ))
    }
}

#[allow(dead_code)]
const _BUDGET_NOTE: Duration = Duration::from_secs(15);

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn s2_mock_path_passes() {
        let ctx = CheckContext {
            kubeconfig: std::path::PathBuf::from("/dev/null"),
            chart_version: "0.6.0".into(),
            namespace: "n".into(),
            timeout: Duration::from_secs(15),
            mock: true,
            redact_logs: false,
        };
        let r = S2L1Swarm.run(&ctx).await.unwrap();
        assert_eq!(r.verdict, crate::checks::CheckVerdict::Pass);
        assert_eq!(r.details["rpcs_succeeded"], 5);
    }

    #[test]
    fn s2_id_and_budget() {
        assert_eq!(S2L1Swarm.id(), SmokeCheckId::S2);
        assert_eq!(S2L1Swarm.budget().as_secs(), 15);
    }

    #[test]
    fn l1_rpc_list_is_5() {
        assert_eq!(L1_RPCS.len(), 5);
        assert_eq!(L1_RPCS[0], "CreateSwarm");
        assert_eq!(L1_RPCS[4], "DeleteSwarm");
    }

    #[test]
    fn p99_budget_frozen_500ms() {
        assert!((P99_BUDGET_MS - 500.0).abs() < f64::EPSILON);
    }
}
