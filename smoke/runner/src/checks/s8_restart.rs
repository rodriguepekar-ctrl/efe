//! PR #23 § 7.8 — **S8 restart resilience** : pod restart resumes
//! within 30 s, no quorum loss, `error_rate < 5 %` during the window.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::json;
use tracing::info;

use crate::checks::{Check, CheckContext, CheckResult, SmokeCheckId};
use crate::error::SmokeResult;

pub struct S8Restart;

/// Max acceptable error rate during the restart window (5 % — § 7.8).
const ERROR_RATE_BUDGET: f64 = 0.05;
/// Max acceptable new-pod ready time (30 s).
const READY_BUDGET_S: f64 = 30.0;

#[async_trait]
impl Check for S8Restart {
    fn id(&self) -> SmokeCheckId {
        SmokeCheckId::S8
    }

    async fn run(&self, ctx: &CheckContext) -> SmokeResult<CheckResult> {
        let started = Instant::now();
        info!(check = "S8", mock = ctx.mock, "S8 restart starting");

        if ctx.mock {
            return Ok(CheckResult::pass(
                self.id(),
                started.elapsed(),
                json!({
                    "mock": true,
                    "new_pod_ready_s": 18.4,
                    "participants_total_post_restart": 3,
                    "error_rate_during": 0.012,
                }),
            ));
        }

        let report = crate::loadgen_client::run_restart_smoke(ctx).await?;
        if report.new_pod_ready_s > READY_BUDGET_S {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!(
                    "new pod Ready in {:.1}s > budget {READY_BUDGET_S}s",
                    report.new_pod_ready_s
                ),
                json!({"new_pod_ready_s": report.new_pod_ready_s}),
            ));
        }
        if report.participants_total_post_restart < 3 {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!(
                    "quorum lost : {} participants after restart (expected 3)",
                    report.participants_total_post_restart
                ),
                json!({"participants_total_post_restart": report.participants_total_post_restart}),
            ));
        }
        if report.error_rate_during > ERROR_RATE_BUDGET {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!(
                    "error_rate during restart {:.4} > budget {ERROR_RATE_BUDGET}",
                    report.error_rate_during
                ),
                json!({"error_rate_during": report.error_rate_during}),
            ));
        }
        Ok(CheckResult::pass(
            self.id(),
            started.elapsed(),
            json!({
                "new_pod_ready_s": report.new_pod_ready_s,
                "participants_total_post_restart": report.participants_total_post_restart,
                "error_rate_during": report.error_rate_during,
            }),
        ))
    }
}

#[allow(dead_code)]
const _BUDGET_NOTE: Duration = Duration::from_secs(30);

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn s8_mock_passes() {
        let ctx = CheckContext {
            kubeconfig: std::path::PathBuf::from("/dev/null"),
            chart_version: "0.6.0".into(),
            namespace: "n".into(),
            timeout: Duration::from_secs(30),
            mock: true,
            redact_logs: false,
        };
        let r = S8Restart.run(&ctx).await.unwrap();
        assert_eq!(r.verdict, crate::checks::CheckVerdict::Pass);
    }

    #[test]
    fn error_rate_budget_5pct() {
        assert!((ERROR_RATE_BUDGET - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn ready_budget_30s() {
        assert!((READY_BUDGET_S - 30.0).abs() < f64::EPSILON);
    }
}
