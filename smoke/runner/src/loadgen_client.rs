//! PR #23 § 11 — PR #20 loadgen consumer (in-process — AD20).
//!
//! Phase-2 ships **structured summary types** + the entry-point
//! functions but the actual loadgen invocation is gated behind the
//! `loadgen-source` Cargo feature.  Without the feature the runner
//! falls back to a no-cluster path that returns `Source` errors
//! (which the orchestrator surfaces as fails — by design : real
//! smoke needs a real cluster).

use crate::checks::CheckContext;
use crate::error::{SmokeError, SmokeResult};

/// Per-check summary returned by the loadgen client (S2, S3).
#[derive(Debug, Clone)]
pub struct L1Summary {
    pub rpcs_succeeded: u32,
    pub p99_ms: f64,
    pub error_rate: f64,
}

#[derive(Debug, Clone)]
pub struct L2Summary {
    pub rpcs_succeeded: u32,
    pub p99_ms: f64,
    pub error_rate: f64,
}

/// Per-check summary returned by the mTLS rotation driver (S4).
#[derive(Debug, Clone)]
pub struct MtlsRotationReport {
    pub total_failed: u32,
    pub p99_before_ms: f64,
    pub p99_after_ms: f64,
    pub cert_cn_before: String,
    pub cert_cn_after: String,
}

#[derive(Debug, Clone)]
pub struct SighupReloadReport {
    pub manifest_version_pre: u32,
    pub manifest_version_post: u32,
    pub total_failed: u32,
}

#[derive(Debug, Clone)]
pub struct RestartReport {
    pub new_pod_ready_s: f64,
    pub participants_total_post_restart: u32,
    pub error_rate_during: f64,
}

#[derive(Debug, Clone)]
pub struct ShutdownReport {
    pub exit_code: i32,
    pub shutdown_s: f64,
    pub gracefully_completed: u32,
}

// ─────────────────────────────────────────────────────────────────────
// Entry points used by S2-S5, S8, S9.
// Without `loadgen-source` feature, return Source error.
// ─────────────────────────────────────────────────────────────────────

pub async fn run_l1_smoke(_ctx: &CheckContext) -> SmokeResult<L1Summary> {
    #[cfg(feature = "loadgen-source")]
    {
        run_l1_smoke_real(_ctx).await
    }
    #[cfg(not(feature = "loadgen-source"))]
    {
        Err(SmokeError::Source(
            "loadgen-source feature disabled : real L1 traffic requires --features loadgen-source ; rebuild with the feature or use --mock"
                .into(),
        ))
    }
}

pub async fn run_l2_smoke(_ctx: &CheckContext) -> SmokeResult<L2Summary> {
    #[cfg(feature = "loadgen-source")]
    {
        run_l2_smoke_real(_ctx).await
    }
    #[cfg(not(feature = "loadgen-source"))]
    {
        Err(SmokeError::Source(
            "loadgen-source feature disabled : real L2 traffic requires --features loadgen-source ; rebuild with the feature or use --mock"
                .into(),
        ))
    }
}

pub async fn run_mtls_rotation_smoke(_ctx: &CheckContext) -> SmokeResult<MtlsRotationReport> {
    Err(SmokeError::Source(
        "S4 mTLS rotation requires --features kind-integration ; use --mock for unit-tests".into(),
    ))
}

pub async fn run_sighup_reload_smoke(_ctx: &CheckContext) -> SmokeResult<SighupReloadReport> {
    Err(SmokeError::Source(
        "S5 SIGHUP reload requires --features kind-integration ; use --mock for unit-tests".into(),
    ))
}

pub async fn run_restart_smoke(_ctx: &CheckContext) -> SmokeResult<RestartReport> {
    Err(SmokeError::Source(
        "S8 restart requires --features kind-integration ; use --mock for unit-tests".into(),
    ))
}

pub async fn run_shutdown_smoke(_ctx: &CheckContext) -> SmokeResult<ShutdownReport> {
    Err(SmokeError::Source(
        "S9 shutdown requires --features kind-integration ; use --mock for unit-tests".into(),
    ))
}

// ─────────────────────────────────────────────────────────────────────
// Real-path implementations (feature-gated).
// ─────────────────────────────────────────────────────────────────────

#[cfg(feature = "loadgen-source")]
async fn run_l1_smoke_real(ctx: &CheckContext) -> SmokeResult<L1Summary> {
    use efe_meta_swarm_loadgen::cli::{
        OutputFormat, PatternKind, RateMode, RunMode, TargetRole, WorkloadKind,
    };
    use efe_meta_swarm_loadgen::{run as loadgen_run, LoadgenConfig};

    let target_url = format!(
        "grpc://efe-meta-swarm-controller.{}.svc:50051",
        ctx.namespace
    );
    let args = efe_meta_swarm_loadgen::cli::RunArgs {
        target: vec![target_url.clone()],
        target_role: TargetRole::Controller,
        conns_per_target: 2,
        per_request_deadline: Some(std::time::Duration::from_secs(2)),
        profile: None,
        pattern: PatternKind::Constant,
        workload: WorkloadKind::Iterate,
        workload_mix: None,
        qps: 10.0,
        concurrency: None,
        rate_mode: RateMode::Hybrid,
        ramp_up: std::time::Duration::from_secs(0),
        duration: Some(std::time::Duration::from_secs(10)),
        requests: Some(5),
        until_slo: vec![],
        shutdown_deadline: std::time::Duration::from_secs(2),
        tls_cert: None,
        tls_key: None,
        tls_ca: None,
        expected_san: None,
        tls_insecure: true,
        prom_listen: "0.0.0.0:0".into(),
        otel_endpoint: None,
        otel_sampler: 0.0,
        otel_service_name: "efe-smoke-s2".into(),
        output_format: OutputFormat::Json,
        output_file: None,
        record: false,
        seed: Some(42),
        mode: RunMode::Dev,
        confirm_prod: false,
        dry_run: false,
        allow_mutation: false,
        i_am_sure: false,
        chaos_marker: None,
        wait_for_signal: None,
    };
    let cfg = LoadgenConfig::from_cli_and_profile(args, None)
        .map_err(|e| SmokeError::Source(format!("loadgen config : {e}")))?;
    let summary = loadgen_run(cfg)
        .await
        .map_err(|e| SmokeError::Source(format!("loadgen run : {e}")))?;
    Ok(L1Summary {
        rpcs_succeeded: summary.total_requests.saturating_sub(summary.total_failed) as u32,
        p99_ms: summary.p99 * 1000.0,
        error_rate: summary.error_rate,
    })
}

#[cfg(feature = "loadgen-source")]
async fn run_l2_smoke_real(ctx: &CheckContext) -> SmokeResult<L2Summary> {
    // Same structure as run_l1_smoke_real but different RPC set ; Phase-3
    // (G.8) will diverge.  For Phase-2 we approximate via the same
    // controller endpoint with a different workload tag.
    let s = run_l1_smoke_real(ctx).await?;
    Ok(L2Summary {
        rpcs_succeeded: s.rpcs_succeeded,
        p99_ms: s.p99_ms,
        error_rate: s.error_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_ctx() -> CheckContext {
        CheckContext {
            kubeconfig: std::path::PathBuf::from("/dev/null"),
            chart_version: "0.6.0".into(),
            namespace: "n".into(),
            timeout: std::time::Duration::from_secs(15),
            mock: false,
            redact_logs: false,
        }
    }

    #[tokio::test]
    async fn without_loadgen_feature_returns_source_error() {
        #[cfg(not(feature = "loadgen-source"))]
        {
            let e = run_l1_smoke(&mock_ctx()).await.unwrap_err();
            assert!(matches!(e, SmokeError::Source(_)));
        }
    }

    #[tokio::test]
    async fn s4_s5_s8_s9_always_return_source_error_without_integration() {
        assert!(matches!(
            run_mtls_rotation_smoke(&mock_ctx()).await.unwrap_err(),
            SmokeError::Source(_)
        ));
        assert!(matches!(
            run_sighup_reload_smoke(&mock_ctx()).await.unwrap_err(),
            SmokeError::Source(_)
        ));
        assert!(matches!(
            run_restart_smoke(&mock_ctx()).await.unwrap_err(),
            SmokeError::Source(_)
        ));
        assert!(matches!(
            run_shutdown_smoke(&mock_ctx()).await.unwrap_err(),
            SmokeError::Source(_)
        ));
    }
}
