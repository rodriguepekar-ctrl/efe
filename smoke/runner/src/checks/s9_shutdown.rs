//! PR #23 § 7.9 — **S9 graceful shutdown** : exit code 0 within 10 s,
//! all 20 in-flight RPCs gracefully completed.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::json;
use tracing::info;

use crate::checks::{Check, CheckContext, CheckResult, SmokeCheckId};
use crate::error::SmokeResult;

pub struct S9Shutdown;

/// Max acceptable shutdown time (10 s — § 7.9).
const SHUTDOWN_BUDGET_S: f64 = 10.0;
/// Expected in-flight RPC count.
const EXPECTED_RPC_COUNT: u32 = 20;

#[async_trait]
impl Check for S9Shutdown {
    fn id(&self) -> SmokeCheckId {
        SmokeCheckId::S9
    }

    async fn run(&self, ctx: &CheckContext) -> SmokeResult<CheckResult> {
        let started = Instant::now();
        info!(check = "S9", mock = ctx.mock, "S9 shutdown starting");

        if ctx.mock {
            return Ok(CheckResult::pass(
                self.id(),
                started.elapsed(),
                json!({
                    "mock": true,
                    "exit_code": 0,
                    "shutdown_s": 6.8,
                    "gracefully_completed": EXPECTED_RPC_COUNT,
                }),
            ));
        }

        let report = crate::loadgen_client::run_shutdown_smoke(ctx).await?;
        if report.exit_code != 0 {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!("controller exit code = {}", report.exit_code),
                json!({"exit_code": report.exit_code}),
            ));
        }
        if report.shutdown_s > SHUTDOWN_BUDGET_S {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!("shutdown took {:.1}s > budget {SHUTDOWN_BUDGET_S}s", report.shutdown_s),
                json!({"shutdown_s": report.shutdown_s}),
            ));
        }
        if report.gracefully_completed < EXPECTED_RPC_COUNT {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!(
                    "only {}/{EXPECTED_RPC_COUNT} in-flight RPCs gracefully completed",
                    report.gracefully_completed
                ),
                json!({"gracefully_completed": report.gracefully_completed}),
            ));
        }
        Ok(CheckResult::pass(
            self.id(),
            started.elapsed(),
            json!({
                "exit_code": 0,
                "shutdown_s": report.shutdown_s,
                "gracefully_completed": report.gracefully_completed,
            }),
        ))
    }
}

#[allow(dead_code)]
const _BUDGET_NOTE: Duration = Duration::from_secs(15);

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn s9_mock_passes() {
        let ctx = CheckContext {
            kubeconfig: std::path::PathBuf::from("/dev/null"),
            chart_version: "0.6.0".into(),
            namespace: "n".into(),
            timeout: Duration::from_secs(15),
            mock: true,
            redact_logs: false,
        };
        let r = S9Shutdown.run(&ctx).await.unwrap();
        assert_eq!(r.verdict, crate::checks::CheckVerdict::Pass);
        assert_eq!(r.details["exit_code"], 0);
    }

    #[test]
    fn shutdown_budget_10s() {
        assert!((SHUTDOWN_BUDGET_S - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn expected_rpc_count_20() {
        assert_eq!(EXPECTED_RPC_COUNT, 20);
    }
}
