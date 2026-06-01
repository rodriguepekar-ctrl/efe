//! PR #23 § 7.5 — **S5 SIGHUP reload** : manifest version increments
//! AND 0 dropped requests.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::json;
use tracing::info;

use crate::checks::{Check, CheckContext, CheckResult, SmokeCheckId};
use crate::error::SmokeResult;

pub struct S5ReloadSighup;

#[async_trait]
impl Check for S5ReloadSighup {
    fn id(&self) -> SmokeCheckId {
        SmokeCheckId::S5
    }

    async fn run(&self, ctx: &CheckContext) -> SmokeResult<CheckResult> {
        let started = Instant::now();
        info!(check = "S5", mock = ctx.mock, "S5 SIGHUP reload starting");

        if ctx.mock {
            return Ok(CheckResult::pass(
                self.id(),
                started.elapsed(),
                json!({
                    "mock": true,
                    "manifest_version_pre": 12,
                    "manifest_version_post": 13,
                    "total_failed": 0,
                }),
            ));
        }

        let report = crate::loadgen_client::run_sighup_reload_smoke(ctx).await?;
        if report.manifest_version_post <= report.manifest_version_pre {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!(
                    "manifest_version did not increment after SIGHUP (pre={}, post={})",
                    report.manifest_version_pre, report.manifest_version_post
                ),
                json!({
                    "manifest_version_pre": report.manifest_version_pre,
                    "manifest_version_post": report.manifest_version_post,
                }),
            ));
        }
        if report.total_failed > 0 {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!("{} dropped in-flight requests during SIGHUP", report.total_failed),
                json!({"total_failed": report.total_failed}),
            ));
        }
        Ok(CheckResult::pass(
            self.id(),
            started.elapsed(),
            json!({
                "manifest_version_pre": report.manifest_version_pre,
                "manifest_version_post": report.manifest_version_post,
                "total_failed": 0,
            }),
        ))
    }
}

#[allow(dead_code)]
const _BUDGET_NOTE: Duration = Duration::from_secs(20);

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn s5_mock_passes() {
        let ctx = CheckContext {
            kubeconfig: std::path::PathBuf::from("/dev/null"),
            chart_version: "0.6.0".into(),
            namespace: "n".into(),
            timeout: Duration::from_secs(20),
            mock: true,
            redact_logs: false,
        };
        let r = S5ReloadSighup.run(&ctx).await.unwrap();
        assert_eq!(r.verdict, crate::checks::CheckVerdict::Pass);
        assert_eq!(r.details["manifest_version_post"], 13);
    }
}
