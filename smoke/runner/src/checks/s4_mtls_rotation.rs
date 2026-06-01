//! PR #23 § 7.4 — **S4 mTLS rotation** : 0 dropped requests during
//! cert switchover.  Anti-T115 — jittered request times.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::json;
use tracing::info;

use crate::checks::{Check, CheckContext, CheckResult, SmokeCheckId};
use crate::error::SmokeResult;

pub struct S4MtlsRotation;

/// Maximum allowed p99 latency increase during rotation (50 ms — § 7.4).
const P99_DELTA_BUDGET_MS: f64 = 50.0;

#[async_trait]
impl Check for S4MtlsRotation {
    fn id(&self) -> SmokeCheckId {
        SmokeCheckId::S4
    }

    async fn run(&self, ctx: &CheckContext) -> SmokeResult<CheckResult> {
        let started = Instant::now();
        info!(check = "S4", mock = ctx.mock, "S4 mTLS rotation starting");

        if ctx.mock {
            return Ok(CheckResult::pass(
                self.id(),
                started.elapsed(),
                json!({
                    "mock": true,
                    "total_failed": 0,
                    "p99_before_ms": 42.0,
                    "p99_after_ms": 47.0,
                    "p99_delta_ms": 5.0,
                    "cert_cn_before": "CA-A",
                    "cert_cn_after": "CA-B",
                }),
            ));
        }

        let report = crate::loadgen_client::run_mtls_rotation_smoke(ctx).await?;

        let p99_delta = report.p99_after_ms - report.p99_before_ms;
        if report.total_failed > 0 {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!("{} dropped requests during cert rotation", report.total_failed),
                json!({"total_failed": report.total_failed, "p99_delta_ms": p99_delta}),
            ));
        }
        if p99_delta > P99_DELTA_BUDGET_MS {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!("p99 delta {p99_delta:.1}ms > budget {P99_DELTA_BUDGET_MS}ms (T115 timing oracle warning)"),
                json!({"p99_delta_ms": p99_delta}),
            ));
        }
        Ok(CheckResult::pass(
            self.id(),
            started.elapsed(),
            json!({
                "total_failed": 0,
                "p99_delta_ms": p99_delta,
                "cert_cn_before": report.cert_cn_before,
                "cert_cn_after": report.cert_cn_after,
            }),
        ))
    }
}

#[allow(dead_code)]
const _BUDGET_NOTE: Duration = Duration::from_secs(25);

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn s4_mock_passes() {
        let ctx = CheckContext {
            kubeconfig: std::path::PathBuf::from("/dev/null"),
            chart_version: "0.6.0".into(),
            namespace: "n".into(),
            timeout: Duration::from_secs(25),
            mock: true,
            redact_logs: false,
        };
        let r = S4MtlsRotation.run(&ctx).await.unwrap();
        assert_eq!(r.verdict, crate::checks::CheckVerdict::Pass);
        assert_eq!(r.details["total_failed"], 0);
        assert_eq!(r.details["cert_cn_before"], "CA-A");
        assert_eq!(r.details["cert_cn_after"], "CA-B");
    }

    #[test]
    fn p99_delta_budget_50ms() {
        assert!((P99_DELTA_BUDGET_MS - 50.0).abs() < f64::EPSILON);
    }
}
