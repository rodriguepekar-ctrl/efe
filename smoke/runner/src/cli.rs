//! PR #23 § 13 + Appendix A — clap CLI (5 subcommands frozen v1.5).

use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};

use crate::checks::SmokeCheckId;
use crate::error::{ExitCode, SmokeError};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "efe-smoke",
    version,
    about = "9 smoke checks — every-PR merge gate (PR #23)",
    long_about = "Boots a cluster, runs 9 closed-taxonomy checks (S1-S9), emits a smoke-result JSON.\n\
                  Hard merge gate.  See docs/EFE_A1_PHASE2_PR23_ARCHITECTURE.md (STABILITY_CONTRACT v1.5 § 23)."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    #[arg(long, global = true, value_enum, default_value = "text")]
    pub log_format: LogFormat,
    #[arg(long, global = true, value_enum, default_value = "info")]
    pub log_level: LogLevel,
}

impl Cli {
    pub fn parse_args() -> Result<Self, CliParseError> {
        Self::try_parse().map_err(CliParseError::from)
    }
}

/// 5 subcommands (STABILITY_CONTRACT v1.5 § 23.4 — frozen).
#[derive(Subcommand, Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Command {
    /// Boot a cluster + run all 9 checks (or a subset via --check).
    Run(RunArgs),
    /// List the 9 frozen check IDs + their budgets.
    List(ListArgs),
    /// Schema-validate a smoke-result.json against v1.
    Validate(ValidateArgs),
    /// Read a smoke-result.json, apply gate criterion, exit 0/1.
    Gate(GateArgs),
    /// Print version + git_sha + stability tier.
    Version,
}

// ─────────────────────────────────────────────────────────────────
//                                run
// ─────────────────────────────────────────────────────────────────
#[derive(Args, Debug, Clone)]
pub struct RunArgs {
    /// Chart version under test (REQUIRED — no default).
    #[arg(long, env = "EFE_CHART_VERSION")]
    pub chart_version: Option<String>,

    /// Provisioner kind (default = kind).
    #[arg(long, value_enum, default_value = "external")]
    pub provisioner: ProvisionerKindArg,

    /// External kubeconfig path (required when `--provisioner=external`).
    #[arg(long, env = "KUBECONFIG")]
    pub kubeconfig: Option<PathBuf>,

    /// Restrict to a subset of checks (repeatable ; default = all 9).
    #[arg(long, value_name = "ID")]
    pub check: Vec<String>,

    /// Keep cluster on completion (debug).
    #[arg(long, action = ArgAction::SetTrue)]
    pub keep_cluster: bool,

    /// Keep cluster only on failure.
    #[arg(long, action = ArgAction::SetTrue)]
    pub keep_cluster_on_failure: bool,

    /// Per-check timeout override (humantime ; default = per-check budget).
    #[arg(long, value_name = "DURATION")]
    pub per_check_timeout: Option<String>,

    /// Global hard cap (humantime ; default = 240 s per AD3).
    #[arg(long, value_name = "DURATION")]
    pub global_timeout: Option<String>,

    /// Output JSON path.
    #[arg(long, value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// PR comment markdown path.
    #[arg(long, value_name = "PATH")]
    pub pr_comment_file: Option<PathBuf>,

    /// gh-pages badge path.
    #[arg(long, value_name = "PATH")]
    pub badge_file: Option<PathBuf>,

    /// T107 / T112 — substitute secret-pattern strings with [REDACTED].
    #[arg(long, action = ArgAction::SetTrue)]
    pub redact_logs: bool,

    /// MOCK path — use in-memory fixtures (no cluster, no network).
    #[arg(long, action = ArgAction::SetTrue)]
    pub mock: bool,

    /// Namespace prefix (used when `--namespace` is NOT supplied — the
    /// orchestrator generates `<prefix>-<8hex>` per run).
    #[arg(long, default_value = "efe-smoke")]
    pub namespace_prefix: String,

    /// **D4** — Explicit namespace override.  When set, the orchestrator
    /// runs all checks against this namespace (no auto-generation, no
    /// auto-cleanup).  Used by CI workflows that pre-install the chart
    /// into a known namespace.
    #[arg(long, value_name = "NAME")]
    pub namespace: Option<String>,
}

// ─────────────────────────────────────────────────────────────────
//                                list
// ─────────────────────────────────────────────────────────────────
#[derive(Args, Debug, Clone)]
pub struct ListArgs {
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}

// ─────────────────────────────────────────────────────────────────
//                              validate
// ─────────────────────────────────────────────────────────────────
#[derive(Args, Debug, Clone)]
pub struct ValidateArgs {
    #[arg(value_name = "PATH")]
    pub input: PathBuf,
}

// ─────────────────────────────────────────────────────────────────
//                                gate
// ─────────────────────────────────────────────────────────────────
#[derive(Args, Debug, Clone)]
pub struct GateArgs {
    #[arg(long, value_name = "PATH")]
    pub input: PathBuf,

    /// AD32 — explicit override.  Requires `--reason`.
    #[arg(long, action = ArgAction::SetTrue)]
    pub allow_skip: bool,

    /// AD33 — reason for `--allow-skip` (required when allow_skip is set).
    #[arg(long, value_name = "TEXT")]
    pub reason: Option<String>,
}

// ─────────────────────────────────────────────────────────────────
//                       enum types
// ─────────────────────────────────────────────────────────────────
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Text,
    Json,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisionerKindArg {
    Kind,
    K3s,
    External,
}

impl From<ProvisionerKindArg> for efe_cluster_provisioner::ProvisionerKind {
    fn from(p: ProvisionerKindArg) -> Self {
        match p {
            ProvisionerKindArg::Kind => Self::Kind,
            ProvisionerKindArg::K3s => Self::K3s,
            ProvisionerKindArg::External => Self::External,
        }
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
}

// ─────────────────────────────────────────────────────────────────
//                       error wrappers
// ─────────────────────────────────────────────────────────────────
#[derive(Debug)]
pub struct CliParseError(pub clap::Error);

impl CliParseError {
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        use clap::error::ErrorKind;
        match self.0.kind() {
            ErrorKind::DisplayHelp
            | ErrorKind::DisplayVersion
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => ExitCode::Success,
            _ => ExitCode::Config,
        }
    }
}

impl From<clap::Error> for CliParseError {
    fn from(e: clap::Error) -> Self {
        let _ = e.print();
        Self(e)
    }
}

impl RunArgs {
    /// Cross-field validation.
    pub fn validate(&self) -> Result<(), SmokeError> {
        if self.chart_version.is_none() {
            return Err(SmokeError::Config(
                "--chart-version is required (or set EFE_CHART_VERSION env)".into(),
            ));
        }
        if matches!(self.provisioner, ProvisionerKindArg::External) && !self.mock && self.kubeconfig.is_none() {
            return Err(SmokeError::Config(
                "--provisioner=external requires --kubeconfig".into(),
            ));
        }
        // Resolve the subset of checks.
        for c in &self.check {
            if SmokeCheckId::parse(c).is_none() {
                return Err(SmokeError::Config(format!(
                    "unknown check id '{c}' (must be S1..S9 or full form S1_boot etc.)"
                )));
            }
        }
        Ok(())
    }

    /// Resolve the subset (or `None` for all 9).
    pub fn selected_checks(&self) -> Option<Vec<SmokeCheckId>> {
        if self.check.is_empty() {
            None
        } else {
            Some(
                self.check
                    .iter()
                    .filter_map(|s| SmokeCheckId::parse(s))
                    .collect(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_compiles() {
        Cli::command().debug_assert();
    }

    #[test]
    fn run_requires_chart_version() {
        let cli = Cli::parse_from(["x", "run"]);
        if let Command::Run(args) = cli.command {
            let e = args.validate().unwrap_err();
            assert_eq!(e.exit_code(), ExitCode::Config);
        }
    }

    #[test]
    fn run_external_requires_kubeconfig() {
        let cli = Cli::parse_from([
            "x", "run",
            "--chart-version", "0.6.0",
            "--provisioner", "external",
        ]);
        if let Command::Run(args) = cli.command {
            let e = args.validate().unwrap_err();
            assert_eq!(e.exit_code(), ExitCode::Config);
        }
    }

    #[test]
    fn run_mock_does_not_require_kubeconfig() {
        let cli = Cli::parse_from([
            "x", "run",
            "--chart-version", "0.6.0",
            "--provisioner", "external",
            "--mock",
        ]);
        if let Command::Run(args) = cli.command {
            args.validate().unwrap();
        } else {
            panic!("expected Run command");
        }
    }

    #[test]
    fn check_filter_accepts_short_and_long() {
        let cli = Cli::parse_from([
            "x", "run",
            "--chart-version", "0.6.0",
            "--mock",
            "--check", "S1",
            "--check", "S6_metrics",
        ]);
        if let Command::Run(args) = cli.command {
            args.validate().unwrap();
            let sel = args.selected_checks().unwrap();
            assert_eq!(sel.len(), 2);
        }
    }

    #[test]
    fn check_filter_rejects_unknown() {
        let cli = Cli::parse_from([
            "x", "run",
            "--chart-version", "0.6.0",
            "--mock",
            "--check", "S10",
        ]);
        if let Command::Run(args) = cli.command {
            let e = args.validate().unwrap_err();
            assert_eq!(e.exit_code(), ExitCode::Config);
        }
    }
}
