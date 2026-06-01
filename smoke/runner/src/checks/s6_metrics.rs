//! PR #23 § 7.6 — **S6 metrics** : 14 STABLE Prom families exposed by
//! the controller scrape (PR #17 STABILITY_CONTRACT v1.0 § 4).

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::json;
use tracing::info;

use crate::checks::{Check, CheckContext, CheckResult, SmokeCheckId};
use crate::error::SmokeResult;
use crate::STABLE_METRIC_FAMILIES;

pub struct S6Metrics;

#[async_trait]
impl Check for S6Metrics {
    fn id(&self) -> SmokeCheckId {
        SmokeCheckId::S6
    }

    async fn run(&self, ctx: &CheckContext) -> SmokeResult<CheckResult> {
        let started = Instant::now();
        info!(check = "S6", mock = ctx.mock, "S6 metrics starting");

        if ctx.mock {
            return Ok(CheckResult::pass(
                self.id(),
                started.elapsed(),
                json!({
                    "mock": true,
                    "controller_families_observed": STABLE_METRIC_FAMILIES.len(),
                    "missing_families": Vec::<String>::new(),
                }),
            ));
        }

        let scrape = crate::metrics_scrape::scrape_controller(ctx).await?;
        let missing = missing_families(&scrape);
        if !missing.is_empty() {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!("missing {} STABLE metric families : {:?}", missing.len(), missing),
                json!({
                    "controller_families_observed": scrape.len(),
                    "missing_families": missing,
                }),
            ));
        }
        Ok(CheckResult::pass(
            self.id(),
            started.elapsed(),
            json!({
                "controller_families_observed": scrape.len(),
                "missing_families": Vec::<String>::new(),
            }),
        ))
    }
}

/// Compare a list of observed family names against the 14 frozen STABLE
/// families ; return the missing-set in stable order.
#[must_use]
pub fn missing_families(observed: &BTreeSet<String>) -> Vec<String> {
    STABLE_METRIC_FAMILIES
        .iter()
        .filter(|f| !observed.contains(**f))
        .map(|f| (*f).to_string())
        .collect()
}

#[allow(dead_code)]
const _BUDGET_NOTE: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn s6_mock_passes() {
        let ctx = CheckContext {
            kubeconfig: std::path::PathBuf::from("/dev/null"),
            chart_version: "0.6.0".into(),
            namespace: "n".into(),
            timeout: Duration::from_secs(10),
            mock: true,
            redact_logs: false,
        };
        let r = S6Metrics.run(&ctx).await.unwrap();
        assert_eq!(r.verdict, crate::checks::CheckVerdict::Pass);
    }

    #[test]
    fn missing_when_observed_is_empty_returns_all_14() {
        let observed = BTreeSet::new();
        let m = missing_families(&observed);
        assert_eq!(m.len(), 14);
    }

    #[test]
    fn missing_when_observed_has_all_14_returns_empty() {
        let observed: BTreeSet<String> = STABLE_METRIC_FAMILIES.iter().map(|s| (*s).to_string()).collect();
        let m = missing_families(&observed);
        assert!(m.is_empty());
    }

    #[test]
    fn missing_returns_exact_diff() {
        let observed: BTreeSet<String> = STABLE_METRIC_FAMILIES
            .iter()
            .skip(1)
            .map(|s| (*s).to_string())
            .collect();
        let m = missing_families(&observed);
        assert_eq!(m, vec!["efe_l2_request_total".to_string()]);
    }
}
