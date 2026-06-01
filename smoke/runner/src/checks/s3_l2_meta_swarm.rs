//! PR #23 § 7.3 — **S3 L2 meta-swarm** : controller↔participant chain
//! works, p99 < 1 s.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::json;
use tracing::info;

use crate::checks::{Check, CheckContext, CheckResult, SmokeCheckId};
use crate::error::SmokeResult;

pub struct S3L2MetaSwarm;

/// Frozen p99 budget (§ 7.3 — 1 s).
const P99_BUDGET_MS: f64 = 1_000.0;

/// L2 RPC catalogue (semantic anchor — see arch doc § 7.3).
pub const L2_RPCS: [&str; 4] = [
    "RegisterParticipant",
    "Heartbeat",
    "ParticipantUpdate",
    "ListParticipants",
];

#[async_trait]
impl Check for S3L2MetaSwarm {
    fn id(&self) -> SmokeCheckId {
        SmokeCheckId::S3
    }

    async fn run(&self, ctx: &CheckContext) -> SmokeResult<CheckResult> {
        let started = Instant::now();
        info!(check = "S3", mock = ctx.mock, "S3 L2 starting");

        if ctx.mock {
            return Ok(CheckResult::pass(
                self.id(),
                started.elapsed(),
                json!({
                    "mock": true,
                    "rpcs_succeeded": 4,
                    "p99_ms": 142.3,
                    "error_rate": 0.0,
                    "chain_complete": true,
                }),
            ));
        }

        let summary = crate::loadgen_client::run_l2_smoke(ctx).await?;
        let p99_ms = summary.p99_ms;
        if summary.rpcs_succeeded < 4 || summary.error_rate > 0.0 {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!(
                    "L2 RPCs : succeeded={}/4, error_rate={}",
                    summary.rpcs_succeeded, summary.error_rate
                ),
                json!({"rpcs_succeeded": summary.rpcs_succeeded, "p99_ms": p99_ms}),
            ));
        }
        if p99_ms > P99_BUDGET_MS {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!("p99 {p99_ms:.1}ms > budget {P99_BUDGET_MS}ms"),
                json!({"rpcs_succeeded": summary.rpcs_succeeded, "p99_ms": p99_ms}),
            ));
        }
        Ok(CheckResult::pass(
            self.id(),
            started.elapsed(),
            json!({"rpcs_succeeded": summary.rpcs_succeeded, "p99_ms": p99_ms, "chain_complete": true}),
        ))
    }
}

#[allow(dead_code)]
const _BUDGET_NOTE: Duration = Duration::from_secs(20);

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn s3_mock_passes() {
        let ctx = CheckContext {
            kubeconfig: std::path::PathBuf::from("/dev/null"),
            chart_version: "0.6.0".into(),
            namespace: "n".into(),
            timeout: Duration::from_secs(20),
            mock: true,
            redact_logs: false,
        };
        let r = S3L2MetaSwarm.run(&ctx).await.unwrap();
        assert_eq!(r.verdict, crate::checks::CheckVerdict::Pass);
    }

    #[test]
    fn p99_budget_1s() {
        assert!((P99_BUDGET_MS - 1000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn l2_rpc_list_is_4() {
        assert_eq!(L2_RPCS.len(), 4);
    }
}
