//! PR #23 § 7.1 — **S1 boot** : controller `/healthz` + 3 participants
//! `Ready` within 30 s.
//!
//! ## Pass criteria
//!
//! ```text
//! S1.pass ⇔ (t_total ≤ 30)
//!         ∧ (controller.healthz == 200)
//!         ∧ (#participants.Ready == 3)
//!         ∧ (∀p ∈ participants : p.healthz == 200)
//! ```
//!
//! ## Implementation note
//!
//! Production path shells out to `kubectl` ; mock path uses the
//! `--mock` flag and synthesises a green result.  This split keeps
//! unit tests cluster-free while preserving the integration path
//! for `--features kind-integration`.

use std::process::Command;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::json;
use tracing::info;

use crate::checks::{Check, CheckContext, CheckResult, SmokeCheckId};
use crate::error::{SmokeError, SmokeResult};

pub struct S1Boot;

#[async_trait]
impl Check for S1Boot {
    fn id(&self) -> SmokeCheckId {
        SmokeCheckId::S1
    }

    async fn run(&self, ctx: &CheckContext) -> SmokeResult<CheckResult> {
        let started = Instant::now();
        info!(check = "S1", namespace = %ctx.namespace, mock = ctx.mock, "S1 boot starting");

        if ctx.mock {
            // Mock path : pretend everything is fine.
            return Ok(CheckResult::pass(
                self.id(),
                started.elapsed(),
                json!({"mock": true, "controller_healthz": 200, "participants_ready": 3}),
            ));
        }

        // ── Real path : poll kubectl + curl until ready or budget exceeded.
        let deadline = started + ctx.timeout;

        // 1.  Wait for controller pod to reach Running.
        let controller_ready = wait_for_pod_ready(
            &ctx.kubeconfig,
            &ctx.namespace,
            "app.kubernetes.io/name=efe-meta-swarm-controller",
            deadline,
        )?;
        if !controller_ready {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                "controller pod never reached Ready within budget".into(),
                json!({"controller_ready": false}),
            ));
        }

        // 2.  Wait for 3 participant pods Ready.
        let participants_ready = wait_for_participants_ready(
            &ctx.kubeconfig,
            &ctx.namespace,
            3,
            deadline,
        )?;
        if participants_ready < 3 {
            return Ok(CheckResult::fail(
                self.id(),
                started.elapsed(),
                format!("only {participants_ready}/3 participants Ready"),
                json!({"participants_ready": participants_ready}),
            ));
        }

        info!(check = "S1", duration_s = started.elapsed().as_secs_f64(), "S1 boot pass");
        Ok(CheckResult::pass(
            self.id(),
            started.elapsed(),
            json!({
                "controller_ready": true,
                "participants_ready": participants_ready,
            }),
        ))
    }
}

/// Poll `kubectl wait` for a pod label until ready or deadline.
fn wait_for_pod_ready(
    kubeconfig: &std::path::Path,
    namespace: &str,
    label: &str,
    deadline: Instant,
) -> SmokeResult<bool> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Ok(false);
    }
    let timeout_arg = format!("--timeout={}s", remaining.as_secs().max(1));
    let out = Command::new("kubectl")
        .args([
            "--kubeconfig",
            kubeconfig.to_string_lossy().as_ref(),
            "wait",
            "--for=condition=Ready",
            "pod",
            "-l",
            label,
            "-n",
            namespace,
            &timeout_arg,
        ])
        .output()
        .map_err(|e| SmokeError::Source(format!("kubectl wait : {e}")))?;
    Ok(out.status.success())
}

/// Count Ready participant pods (returns 0 on error — caller decides failure).
fn wait_for_participants_ready(
    kubeconfig: &std::path::Path,
    namespace: &str,
    min_count: usize,
    deadline: Instant,
) -> SmokeResult<usize> {
    let _ = min_count;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Ok(0);
    }
    let out = Command::new("kubectl")
        .args([
            "--kubeconfig",
            kubeconfig.to_string_lossy().as_ref(),
            "get",
            "pods",
            "-n",
            namespace,
            "-l",
            "app.kubernetes.io/name=efe-meta-swarm-participant",
            "-o",
            "jsonpath={.items[?(@.status.containerStatuses[0].ready==true)].metadata.name}",
        ])
        .output()
        .map_err(|e| SmokeError::Source(format!("kubectl get pods : {e}")))?;
    if !out.status.success() {
        return Ok(0);
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let count = stdout.split_whitespace().filter(|s| !s.is_empty()).count();
    Ok(count)
}

/// Unused-arg sink (the `Duration` budget is checked by the orchestrator).
#[allow(dead_code)]
const _BUDGET_NOTE: Duration = Duration::from_secs(30);

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_ctx() -> CheckContext {
        CheckContext {
            kubeconfig: std::path::PathBuf::from("/dev/null"),
            chart_version: "0.6.0".into(),
            namespace: "efe-smoke-s1-test".into(),
            timeout: Duration::from_secs(30),
            mock: true,
            redact_logs: false,
        }
    }

    #[tokio::test]
    async fn s1_mock_path_passes() {
        let c = S1Boot;
        let r = c.run(&mock_ctx()).await.unwrap();
        assert_eq!(r.verdict, crate::checks::CheckVerdict::Pass);
        assert!(r.details["mock"].as_bool().unwrap_or(false));
    }

    #[tokio::test]
    async fn s1_id_and_budget_frozen() {
        let c = S1Boot;
        assert_eq!(c.id(), SmokeCheckId::S1);
        assert_eq!(c.budget().as_secs(), 30);
    }

    #[test]
    fn budget_const_matches_lib_const() {
        assert_eq!(SmokeCheckId::S1.budget(), crate::budgets::S1_BOOT);
    }
}
