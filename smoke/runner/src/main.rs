//! `efe-smoke` binary entry — PR #23 § 8.
//!
//! D1 fix : artefacts (JSON / MD / SVG) are emitted **before** deciding
//! the process exit code, so CI workflows can post the PR comment even
//! on smoke fail.
//!
//! D2 fix : `provisioner.teardown()` is always invoked in a deferred
//! block (regardless of pass/fail), preventing kind-cluster leakage.

use std::process::ExitCode as StdExitCode;
use std::time::Duration;

use efe_cluster_provisioner::ClusterProvisioner;
use efe_smoke::cli::{Cli, Command, GateArgs, ListArgs, RunArgs, ValidateArgs};
use efe_smoke::error::{ExitCode, SmokeError, SmokeResult};
use efe_smoke::gate::GateInput;
use efe_smoke::orchestrator::{SmokeConfig, SmokeOrchestrator};
use efe_smoke::result::{read_report_json, write_report_json, SmokeReport};
use tracing::{error, info, warn};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> StdExitCode {
    let cli = match Cli::parse_args() {
        Ok(c) => c,
        Err(err) => {
            let code = err.exit_code();
            std::process::exit(code.as_i32());
        }
    };
    init_logging(&cli);

    let result = match cli.command {
        Command::Run(args) => dispatch_run(args).await,
        Command::List(args) => dispatch_list(args),
        Command::Validate(args) => dispatch_validate(args),
        Command::Gate(args) => dispatch_gate(args),
        Command::Version => {
            println!("{}", efe_smoke::build::long());
            return StdExitCode::SUCCESS;
        }
    };

    let exit = match result {
        Ok(ec) => ec,
        Err(err) => {
            error!(error = %err, "efe-smoke aborted");
            err.exit_code()
        }
    };
    StdExitCode::from(u8::try_from(exit.as_i32()).unwrap_or(5))
}

fn init_logging(cli: &Cli) {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(cli.log_level.as_str()));
    let r = tracing_subscriber::registry().with(filter);
    match cli.log_format {
        efe_smoke::cli::LogFormat::Text => {
            r.with(fmt::layer().with_target(true).compact()).init();
        }
        efe_smoke::cli::LogFormat::Json => {
            r.with(fmt::layer().json().with_target(true).with_current_span(true)).init();
        }
    }
}

async fn dispatch_run(args: RunArgs) -> SmokeResult<ExitCode> {
    args.validate()?;

    let chart_version = args.chart_version.clone().unwrap();
    let mut config = SmokeConfig::defaults(chart_version.clone());
    config.namespace_prefix = args.namespace_prefix.clone();
    config.keep_cluster = args.keep_cluster;
    config.keep_cluster_on_failure = args.keep_cluster_on_failure;
    config.redact_logs = args.redact_logs;
    config.mock = args.mock;
    config.only_checks = args.selected_checks();
    if let Some(ns) = &args.namespace {
        // D4 — operator-managed namespace : no auto-cleanup.
        config.namespace_override = Some(ns.clone());
        config.manage_namespace = false;
    }
    if let Some(s) = &args.per_check_timeout {
        config.per_check_timeout_override = Some(parse_duration(s)?);
    }
    if let Some(s) = &args.global_timeout {
        config.global_timeout = parse_duration(s)?;
    }

    // Provision a cluster (kind/k3s/external) unless we're in mock mode.
    // The provisioner is held until the end of this fn so its `teardown`
    // can run after orchestrator+artefact emission (D2 fix).
    let (kubeconfig, provisioner): (std::path::PathBuf, Option<Box<dyn ClusterProvisioner>>) =
        if args.mock {
            (std::path::PathBuf::from("/dev/null"), None)
        } else {
            let p = efe_cluster_provisioner::from_kind(
                args.provisioner.into(),
                args.kubeconfig.clone(),
                format!("efe-smoke-{}", uuid::Uuid::now_v7().simple()),
            );
            let kc = p.provision().await?;
            (kc, Some(p))
        };

    // ── Always run orchestrator + always emit artefacts + always
    //    teardown.  D1+D2 — even on fail.
    let orchestrator = SmokeOrchestrator::new(kubeconfig, config);
    let report_result = orchestrator.run().await;

    // Emit artefacts regardless of verdict (D1 guarantee).
    let report = match &report_result {
        Ok(r) => Some(r.clone()),
        Err(_) => None,
    };
    if let Some(r) = &report {
        if let Err(e) = emit_artefacts(r, &args) {
            warn!(error = %e, "artefact emission failed (continuing)");
        }
    }

    // Teardown the provisioner (D2 fix — best-effort).
    if let Some(p) = &provisioner {
        let keep = args.keep_cluster
            || (args.keep_cluster_on_failure
                && report.as_ref().is_some_and(|r| {
                    matches!(r.overall_verdict, efe_smoke::checks::CheckVerdict::Fail)
                }));
        if !keep {
            if let Err(e) = p.teardown().await {
                warn!(error = %e, "provisioner teardown failed (best-effort)");
            } else {
                info!("provisioner teardown ok");
            }
        }
    }

    // Final exit decision (D1 — based on the report verdict, not on
    // whether `run` returned Ok or Err).
    let report = report_result?;
    if matches!(report.overall_verdict, efe_smoke::checks::CheckVerdict::Fail) {
        return Err(SmokeError::SmokeVerdict {
            failed_count: report.failed_count(),
        });
    }
    Ok(ExitCode::Success)
}

fn emit_artefacts(report: &SmokeReport, args: &RunArgs) -> SmokeResult<()> {
    // D3 — apply redaction to the report's free-form fields before
    // emission, if the operator requested it.
    let report = if args.redact_logs {
        redact_report(report)
    } else {
        report.clone()
    };
    if let Some(out) = &args.output {
        write_report_json(&report, out)?;
        info!(path = %out.display(), "smoke result JSON written");
    }
    if let Some(path) = &args.pr_comment_file {
        let body = efe_smoke::result::markdown::build_pr_comment(&report);
        std::fs::write(path, body).map_err(|source| SmokeError::Io {
            path: path.clone(),
            source,
        })?;
        info!(path = %path.display(), "PR comment markdown written");
    }
    if let Some(path) = &args.badge_file {
        let svg =
            efe_smoke::result::badge::render(report.overall_verdict, report.failed_count());
        std::fs::write(path, svg).map_err(|source| SmokeError::Io {
            path: path.clone(),
            source,
        })?;
        info!(path = %path.display(), "smoke badge SVG written");
    }
    Ok(())
}

/// D3 — scrub free-form text fields in a report.
fn redact_report(report: &SmokeReport) -> SmokeReport {
    use efe_smoke::redact::{scrub, scrub_json};
    let mut scrubbed = report.clone();
    for (_id, check) in scrubbed.checks.iter_mut() {
        check.error_summary = scrub(&check.error_summary);
        check.details = scrub_json(&check.details);
    }
    scrubbed
}

fn dispatch_list(args: ListArgs) -> SmokeResult<ExitCode> {
    use efe_smoke::checks::SmokeCheckId;
    match args.format {
        efe_smoke::cli::OutputFormat::Json => {
            let entries: Vec<_> = SmokeCheckId::all()
                .iter()
                .map(|id| {
                    serde_json::json!({
                        "id": id.as_str(),
                        "short": id.short(),
                        "budget_s": id.budget().as_secs(),
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&entries)?);
        }
        efe_smoke::cli::OutputFormat::Text => {
            println!("{:24} {:>10} short", "id", "budget(s)");
            for id in SmokeCheckId::all() {
                println!(
                    "{:24} {:>10} {}",
                    id.as_str(),
                    id.budget().as_secs(),
                    id.short()
                );
            }
        }
    }
    Ok(ExitCode::Success)
}

fn dispatch_validate(args: ValidateArgs) -> SmokeResult<ExitCode> {
    let report = read_report_json(&args.input)?;
    efe_smoke::result::json::validate_schema(&report)?;
    println!("[validate] OK : {} ({} checks)", args.input.display(), report.checks.len());
    Ok(ExitCode::Success)
}

fn dispatch_gate(args: GateArgs) -> SmokeResult<ExitCode> {
    efe_smoke::gate::decide(GateInput {
        report_path: &args.input,
        allow_skip: args.allow_skip,
        reason: args.reason.as_deref(),
    })
}

fn parse_duration(s: &str) -> SmokeResult<Duration> {
    humantime::parse_duration(s)
        .map_err(|e| SmokeError::Config(format!("invalid duration '{s}' : {e}")))
}
