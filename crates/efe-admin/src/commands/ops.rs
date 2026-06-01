//! Sprint Φ-D-OPS (D99) — `efe-admin ops` subcommand surface.
//!
//! Exposes the OPS surface to operators directly :
//!
//!   * `efe-admin ops gate`       — run the master gate locally.
//!   * `efe-admin ops status`     — fetch the latest master report.
//!   * `efe-admin ops dr-drill`   — trigger a DR drill (dry by default).
//!   * `efe-admin ops chargeback` — print the current chargeback rollup.
//!
//! Each variant is a thin wrapper around the corresponding python
//! script under `scripts/ops/` so the CLI stays "1 tool = 1 entry"
//! and the backend logic remains in python.
//!
//! Cf. `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 15.

use std::path::PathBuf;
use std::process::Command;

use clap::{Args, Subcommand as ClapSubcommand};

use crate::context::AdminContext;
use crate::error::AdminError;

/// Top-level args for the `ops` subcommand.
#[derive(Debug, Args)]
pub struct OpsArgs {
    #[command(subcommand)]
    pub command: OpsCommand,
}

#[derive(Debug, ClapSubcommand)]
pub enum OpsCommand {
    /// Run the master SLO gate locally and print the verdict.
    Gate(OpsGateArgs),
    /// Fetch the latest master report (= last 24 h).
    Status(OpsStatusArgs),
    /// Trigger a DR drill simulator run.
    DrDrill(OpsDrDrillArgs),
    /// Print or export the latest chargeback rollup.
    Chargeback(OpsChargebackArgs),
    /// Audit the tenant FSM transitions matrix.
    FsmAudit,
    /// Check runbook reference autolinks.
    RunbookAudit,
}

#[derive(Debug, Args)]
pub struct OpsGateArgs {
    /// Tolerate SLOs marked as deferred (= SKIP-DT).
    #[arg(long, default_value_t = true)]
    pub tolerate_skip_with_dt: bool,
    /// Override the master baseline path.
    #[arg(long)]
    pub master: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct OpsStatusArgs {
    /// Output the report to this path (default : stdout).
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct OpsDrDrillArgs {
    #[arg(long, default_value = "dry")]
    pub mode: String,
    #[arg(long, default_value = "eu-west-3")]
    pub region_from: String,
    #[arg(long, default_value = "eu-west-1")]
    pub region_to: String,
    /// Mandatory when `--mode prod`.
    #[arg(long)]
    pub prod_confirm: bool,
}

#[derive(Debug, Args)]
pub struct OpsChargebackArgs {
    /// Run against the demo dataset (= no production access).
    #[arg(long, default_value_t = true)]
    pub demo: bool,
    /// Output file (default `target/ops/chargeback.json`).
    #[arg(long)]
    pub out: Option<PathBuf>,
}

/// Dispatch entrypoint called from `main.rs`.
///
/// The actual logic is delegated to the python scripts ; this wrapper
/// just ensures the operator gets a single binary surface.
pub async fn dispatch(args: OpsArgs, _ctx: &AdminContext) -> Result<(), AdminError> {
    match args.command {
        OpsCommand::Gate(a) => run_gate(a).await,
        OpsCommand::Status(a) => run_status(a).await,
        OpsCommand::DrDrill(a) => run_dr_drill(a).await,
        OpsCommand::Chargeback(a) => run_chargeback(a).await,
        OpsCommand::FsmAudit => run_python("scripts/ops/tenant_state_machine_audit.py", &[]),
        OpsCommand::RunbookAudit => run_python("scripts/ops/runbook_auto_link.py", &[]),
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────

fn run_python(script: &str, extra_args: &[&str]) -> Result<(), AdminError> {
    let status = Command::new("python3")
        .arg(script)
        .args(extra_args)
        .status()
        .map_err(|e| AdminError::Internal(format!("spawn python3: {e}")))?;
    if !status.success() {
        return Err(AdminError::backend(
            "ops_script_failed",
            format!("script {script} exited {status}"),
        ));
    }
    Ok(())
}

async fn run_gate(args: OpsGateArgs) -> Result<(), AdminError> {
    let master = args
        .master
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "bench_baselines/v0.9.0/ops_master_slos.yaml".to_string());
    // 1. Consolidate.
    run_python(
        "scripts/ops/consolidate_all_slos.py",
        &[
            "--baselines", master.as_str(),
            "--master",    master.as_str(),
            "--measures",  "target/ops/measures.json",
            "--out",       "target/ops/master_report.md",
            "--out-json",  "target/ops/master_report.json",
        ],
    )?;
    // 2. Gate.
    let mut gate_args = vec!["--report", "target/ops/master_report.json"];
    if args.tolerate_skip_with_dt {
        gate_args.push("--tolerate-skip-with-dt");
    }
    run_python("scripts/ops/master_gate_check.py", &gate_args)
}

async fn run_status(args: OpsStatusArgs) -> Result<(), AdminError> {
    let report_path = std::path::Path::new("target/ops/master_report.md");
    if !report_path.exists() {
        return Err(AdminError::backend(
            "no_report",
            "no master_report.md present ; run `efe-admin ops gate` first",
        ));
    }
    let content = std::fs::read_to_string(report_path)?;
    match args.out {
        Some(p) => {
            std::fs::write(&p, &content)?;
            println!("wrote {}", p.display());
        }
        None => print!("{content}"),
    }
    Ok(())
}

async fn run_dr_drill(args: OpsDrDrillArgs) -> Result<(), AdminError> {
    let mut argv: Vec<String> = vec![
        "--mode".to_string(), args.mode.clone(),
        "--region-from".to_string(), args.region_from.clone(),
        "--region-to".to_string(),   args.region_to.clone(),
    ];
    if args.prod_confirm {
        argv.push("--prod-confirm".to_string());
    }
    let argv_ref: Vec<&str> = argv.iter().map(String::as_str).collect();
    run_python("scripts/ops/dr_drill_simulator.py", &argv_ref)
}

async fn run_chargeback(args: OpsChargebackArgs) -> Result<(), AdminError> {
    let out = args
        .out
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "target/ops/chargeback.json".to_string());
    let mut argv = vec!["--out", out.as_str()];
    if args.demo {
        argv.push("--demo");
    }
    run_python("scripts/ops/chargeback_compute.py", &argv)
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Helper : a tiny stub CLI with just our subcommand for arg parsing tests.
    #[derive(Debug, Parser)]
    struct StubCli {
        #[command(subcommand)]
        command: OpsCommand,
    }

    #[test]
    fn ops_gate_parses() {
        let cli = StubCli::try_parse_from(["ops", "gate"]).expect("parse");
        match cli.command {
            OpsCommand::Gate(_) => {}
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn ops_dr_drill_defaults() {
        let cli = StubCli::try_parse_from(["ops", "dr-drill"]).expect("parse");
        match cli.command {
            OpsCommand::DrDrill(a) => {
                assert_eq!(a.mode, "dry");
                assert_eq!(a.region_from, "eu-west-3");
                assert_eq!(a.region_to, "eu-west-1");
                assert!(!a.prod_confirm);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn ops_chargeback_demo_by_default() {
        let cli = StubCli::try_parse_from(["ops", "chargeback"]).expect("parse");
        match cli.command {
            OpsCommand::Chargeback(a) => assert!(a.demo),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn ops_fsm_audit_no_args() {
        let cli = StubCli::try_parse_from(["ops", "fsm-audit"]).expect("parse");
        assert!(matches!(cli.command, OpsCommand::FsmAudit));
    }
}
