//! PR #23 § 7.7 — **S7 OTel** : trace has ≥ 2 spans, both expected
//! service names.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::json;
use tracing::info;

use crate::checks::{Check, CheckContext, CheckResult, SmokeCheckId};
use crate::error::SmokeResult;

pub struct S7Otel;

/// Expected service names in the propagation chain.
pub const REQUIRED_SERVICES: [&str; 2] = [
    "efe-meta-swarm-controller",
    "efe-meta-swarm-participant",
];

/// Sanity upper bound on chain length (anti-recursion — § 7.7).
pub const MAX_CHAIN_LEN: usize = 4;

#[async_trait]
impl Check for S7Otel {
    fn id(&self) -> SmokeCheckId {
        SmokeCheckId::S7
    }

    async fn run(&self, ctx: &CheckContext) -> SmokeResult<CheckResult> {
        let started = Instant::now();
        info!(check = "S7", mock = ctx.mock, "S7 OTel starting");

        if ctx.mock {
            return Ok(CheckResult::pass(
                self.id(),
                started.elapsed(),
                json!({
                    "mock": true,
                    "spans": 2,
                    "services": REQUIRED_SERVICES,
                }),
            ));
        }

        let chain = crate::otel_collector::probe_trace_chain(ctx).await?;
        let services_present: Vec<&str> = REQUIRED_SERVICES
            .iter()
            .filter(|s| chain.services.iter().any(|x| x == *s))
            .copied()
            .collect();

        if chain.spans < 2 {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!("OTel chain too short : {} spans (need ≥ 2)", chain.spans),
                json!({"spans": chain.spans}),
            ));
        }
        if services_present.len() != REQUIRED_SERVICES.len() {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!(
                    "missing required services in chain : observed={:?}, required={:?}",
                    chain.services, REQUIRED_SERVICES
                ),
                json!({
                    "spans": chain.spans,
                    "services": chain.services,
                    "missing": REQUIRED_SERVICES
                        .iter()
                        .filter(|s| !chain.services.iter().any(|x| x == *s))
                        .collect::<Vec<_>>(),
                }),
            ));
        }
        if chain.spans > MAX_CHAIN_LEN {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!("OTel chain too long : {} spans (max {MAX_CHAIN_LEN})", chain.spans),
                json!({"spans": chain.spans}),
            ));
        }
        Ok(CheckResult::pass(
            self.id(),
            started.elapsed(),
            json!({"spans": chain.spans, "services": chain.services}),
        ))
    }
}

#[allow(dead_code)]
const _BUDGET_NOTE: Duration = Duration::from_secs(15);

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn s7_mock_passes() {
        let ctx = CheckContext {
            kubeconfig: std::path::PathBuf::from("/dev/null"),
            chart_version: "0.6.0".into(),
            namespace: "n".into(),
            timeout: Duration::from_secs(15),
            mock: true,
            redact_logs: false,
        };
        let r = S7Otel.run(&ctx).await.unwrap();
        assert_eq!(r.verdict, crate::checks::CheckVerdict::Pass);
        assert_eq!(r.details["spans"], 2);
    }

    #[test]
    fn required_services_locked() {
        assert_eq!(REQUIRED_SERVICES.len(), 2);
        assert!(REQUIRED_SERVICES.contains(&"efe-meta-swarm-controller"));
        assert!(REQUIRED_SERVICES.contains(&"efe-meta-swarm-participant"));
    }

    #[test]
    fn max_chain_len_4() {
        assert_eq!(MAX_CHAIN_LEN, 4);
    }
}
