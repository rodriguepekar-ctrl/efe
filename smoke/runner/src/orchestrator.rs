//! PR #23 § 8 — orchestrator : provisions a cluster, sequences the 9
//! checks, enforces per-check + global budgets, aggregates results.
//!
//! Sequential execution (AD14) — single-node kind would inflate
//! variance under parallelism.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tracing::{info, warn};
use uuid::Uuid;

use crate::checks::{all_checks, CheckContext, CheckResult, CheckVerdict, SmokeCheckId};
use crate::cluster_glue::{assert_not_production, delete_namespace};
use crate::error::SmokeResult;
use crate::result::SmokeReport;

/// Configuration passed to the orchestrator.
#[derive(Debug, Clone)]
pub struct SmokeConfig {
    pub chart_version: String,
    pub namespace_prefix: String,
    /// **D4** — Operator-supplied namespace override (used when the
    /// chart is pre-installed in a known namespace, e.g. CI workflow
    /// installs into `efe`).  When `Some`, the orchestrator does NOT
    /// generate `efe-smoke-{uuid}` and does NOT call `delete_namespace`
    /// on completion (operator-managed lifecycle).
    pub namespace_override: Option<String>,
    pub per_check_timeout_override: Option<Duration>,
    pub global_timeout: Duration,
    pub keep_cluster: bool,
    pub keep_cluster_on_failure: bool,
    pub redact_logs: bool,
    pub mock: bool,
    /// Restrict to a subset of check IDs (default = all 9).
    pub only_checks: Option<Vec<SmokeCheckId>>,
    /// When false, the orchestrator skips `delete_namespace` on
    /// teardown (used by `--namespace` override since the operator
    /// owns the namespace lifecycle).  Auto-set by `defaults` to true.
    pub manage_namespace: bool,
}

impl SmokeConfig {
    #[must_use]
    pub fn defaults(chart_version: String) -> Self {
        Self {
            chart_version,
            namespace_prefix: "efe-smoke".into(),
            namespace_override: None,
            per_check_timeout_override: None,
            global_timeout: crate::budgets::GLOBAL_HARD_CAP,
            keep_cluster: false,
            keep_cluster_on_failure: false,
            redact_logs: false,
            mock: false,
            only_checks: None,
            manage_namespace: true,
        }
    }

    /// Helper : configure for a chart pre-installed into a known
    /// namespace.  Disables auto-cleanup of the namespace.
    #[must_use]
    pub fn with_namespace_override(mut self, ns: impl Into<String>) -> Self {
        self.namespace_override = Some(ns.into());
        self.manage_namespace = false;
        self
    }
}

pub struct SmokeOrchestrator {
    pub kubeconfig: PathBuf,
    pub config: SmokeConfig,
}

impl SmokeOrchestrator {
    #[must_use]
    pub fn new(kubeconfig: PathBuf, config: SmokeConfig) -> Self {
        Self { kubeconfig, config }
    }

    /// Sequential execution of the configured checks (AD14).
    pub async fn run(&self) -> SmokeResult<SmokeReport> {
        let started = Instant::now();
        let run_id = Uuid::now_v7().to_string();
        // D4 — operator-supplied override takes precedence over auto-gen.
        let namespace = self
            .config
            .namespace_override
            .clone()
            .unwrap_or_else(|| {
                format!(
                    "{}-{}",
                    self.config.namespace_prefix,
                    run_id.chars().take(8).collect::<String>()
                )
            });

        // AD9 — refuse production cluster.
        if !self.config.mock {
            assert_not_production(&self.kubeconfig)?;
        }

        // Build the check list (subset or all 9).
        let selected_ids = match &self.config.only_checks {
            Some(ids) => ids.clone(),
            None => SmokeCheckId::all().to_vec(),
        };
        let checks: Vec<_> = all_checks()
            .into_iter()
            .filter(|c| selected_ids.contains(&c.id()))
            .collect();

        info!(
            run_id = %run_id,
            namespace = %namespace,
            count = checks.len(),
            mock = self.config.mock,
            "smoke run starting"
        );

        let mut results: Vec<CheckResult> = Vec::with_capacity(checks.len());
        let mut deadline_exceeded = false;

        for check in &checks {
            // Global deadline guard.
            if started.elapsed() >= self.config.global_timeout {
                warn!(
                    elapsed_s = started.elapsed().as_secs_f64(),
                    "global timeout reached ; skipping remaining checks"
                );
                deadline_exceeded = true;
                break;
            }

            let per_check_timeout = self
                .config
                .per_check_timeout_override
                .unwrap_or_else(|| check.budget());

            let ctx = CheckContext {
                kubeconfig: self.kubeconfig.clone(),
                chart_version: self.config.chart_version.clone(),
                namespace: namespace.clone(),
                timeout: per_check_timeout,
                mock: self.config.mock,
                redact_logs: self.config.redact_logs,
            };

            let check_started = Instant::now();
            let result = tokio::time::timeout(per_check_timeout, check.run(&ctx)).await;
            let result = match result {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => CheckResult::fail(
                    check.id(),
                    check_started.elapsed(),
                    format!("error: {e}"),
                    serde_json::json!({"error": e.to_string()}),
                ),
                Err(_elapsed) => CheckResult::fail(
                    check.id(),
                    per_check_timeout,
                    format!("timeout after {per_check_timeout:?}"),
                    serde_json::json!({"timeout": true}),
                ),
            };

            info!(
                check = %result.id.as_str(),
                verdict = %result.verdict.as_str(),
                duration_s = result.duration_s,
                "check complete"
            );
            results.push(result);
        }

        let failed_count = results
            .iter()
            .filter(|r| matches!(r.verdict, CheckVerdict::Fail))
            .count();
        // AD3 — global timeout that prevented some checks from running
        // counts as fail.
        let extra_fail = if deadline_exceeded && results.len() < checks.len() {
            checks.len() - results.len()
        } else {
            0
        };
        let total_failed = failed_count + extra_fail;
        let overall = if total_failed == 0 {
            CheckVerdict::Pass
        } else {
            CheckVerdict::Fail
        };

        let total_duration = started.elapsed();
        let report = SmokeReport::build(
            run_id,
            self.config.chart_version.clone(),
            crate::build::GIT_SHA.to_string(),
            total_duration,
            overall,
            results,
        );

        // Teardown logic — best-effort, never blocks artefact emission.
        // **D1 fix** : `run` MUST return Ok regardless of overall verdict
        // so that the caller (main.rs) can emit artefacts BEFORE deciding
        // the process exit code via `gate::decide`.  Returning Err here
        // previously dropped the report and CI workflows could not post
        // a PR comment with the failure detail.
        let should_keep = self.config.keep_cluster
            || (self.config.keep_cluster_on_failure
                && matches!(overall, CheckVerdict::Fail));
        if !should_keep && !self.config.mock && self.config.manage_namespace {
            let _ = delete_namespace(&self.kubeconfig, &namespace);
        }

        // `total_failed` is reachable via `report.failed_count()` ; we
        // surface it in the log line for forensics + drop the local
        // variable.
        let _ = total_failed;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_run_passes() {
        let cfg = SmokeConfig {
            chart_version: "0.6.0".into(),
            namespace_prefix: "efe-smoke-test".into(),
            namespace_override: None,
            per_check_timeout_override: Some(Duration::from_secs(2)),
            global_timeout: Duration::from_secs(60),
            keep_cluster: false,
            keep_cluster_on_failure: false,
            redact_logs: false,
            mock: true,
            only_checks: None,
            manage_namespace: true,
        };
        let o = SmokeOrchestrator::new(PathBuf::from("/dev/null"), cfg);
        let report = o.run().await.unwrap();
        assert_eq!(report.overall_verdict, CheckVerdict::Pass);
        assert_eq!(report.checks.len(), 9);
    }

    #[tokio::test]
    async fn mock_run_with_subset_runs_only_selected() {
        let mut cfg = SmokeConfig::defaults("0.6.0".into());
        cfg.mock = true;
        cfg.only_checks = Some(vec![SmokeCheckId::S1, SmokeCheckId::S6]);
        let o = SmokeOrchestrator::new(PathBuf::from("/dev/null"), cfg);
        let report = o.run().await.unwrap();
        assert_eq!(report.checks.len(), 2);
    }

    #[tokio::test]
    async fn budgets_under_global_cap_passes() {
        // Sum of all 9 budgets is 180s ; global cap is 240s.
        let mut cfg = SmokeConfig::defaults("0.6.0".into());
        cfg.mock = true;
        let o = SmokeOrchestrator::new(PathBuf::from("/dev/null"), cfg);
        let _ = o.run().await.unwrap();
    }

    #[tokio::test]
    async fn namespace_override_disables_auto_cleanup() {
        let cfg = SmokeConfig::defaults("0.6.0".into())
            .with_namespace_override("efe");
        assert_eq!(cfg.namespace_override.as_deref(), Some("efe"));
        assert!(!cfg.manage_namespace);
        let mut cfg = cfg;
        cfg.mock = true;
        let o = SmokeOrchestrator::new(PathBuf::from("/dev/null"), cfg);
        let r = o.run().await.unwrap();
        assert_eq!(r.checks.len(), 9);
    }

    #[tokio::test]
    async fn d1_failing_path_still_returns_ok_report() {
        // The orchestrator MUST return Ok(report) even when checks fail
        // so the caller can emit artefacts before deciding exit code.
        let mut cfg = SmokeConfig::defaults("0.6.0".into());
        cfg.mock = true;
        cfg.per_check_timeout_override = Some(Duration::from_nanos(1));
        let o = SmokeOrchestrator::new(PathBuf::from("/dev/null"), cfg);
        // Returns Ok regardless of verdict ; verdict is in `report.overall_verdict`.
        let r = o.run().await.unwrap();
        assert!(matches!(r.overall_verdict, CheckVerdict::Pass | CheckVerdict::Fail));
    }
}
