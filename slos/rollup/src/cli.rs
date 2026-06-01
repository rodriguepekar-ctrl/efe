//! PR #22 § 15 + Appendix A — clap CLI (7 subcommands frozen v1.4).

use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};

use crate::error::{ExitCode, SloError};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "efe-slo-rollup",
    version,
    about = "EFE 4-SLO surface + multi-tier rollup compute (PR #22)",
    long_about = "Computes 4 SLOs, rolls up through 8-tier windows, publishes signed quarterly evidence.\n\
                  See docs/EFE_A1_PHASE2_PR22_ARCHITECTURE.md (STABILITY_CONTRACT v1.4 § 22)."
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

/// 7 subcommands (STABILITY_CONTRACT v1.4 § 22.5 — frozen).
#[derive(Subcommand, Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Command {
    /// Compute SLI(s) for a window/period — used by T0/T1 jobs.
    Compute(ComputeArgs),
    /// Run a full T1/T2/T3 stage.
    Rollup(RollupArgs),
    /// Build the public site (mdbook + JSON API + badges).
    Publish(PublishArgs),
    /// Sign or verify a quarterly evidence PDF.
    Evidence(EvidenceArgs),
    /// List computed SLIs (range query).
    List(ListArgs),
    /// Schema-validate stored files.
    Validate(ValidateArgs),
    /// Print version + git_sha + rustc_version.
    Version,
}

// ─────────────────────────────────────────────────────────────────
//                              compute
// ─────────────────────────────────────────────────────────────────
#[derive(Args, Debug, Clone)]
pub struct ComputeArgs {
    /// Subset of SLO IDs to compute (default = all 4).  Repeatable.
    #[arg(long, value_name = "SLO_ID")]
    pub slo: Vec<String>,
    /// Window size (humantime — `5m`, `1h`, `24h`, etc.).
    #[arg(long, value_name = "DURATION", default_value = "1h")]
    pub window: String,
    /// Negative offset from now (humantime).  `--offset 1h` means
    /// "compute the previous hour".
    #[arg(long, value_name = "DURATION", default_value = "0s")]
    pub offset: String,
    /// Backfill a historical hour (requires `--release-backfill-confirmed`).
    #[arg(long, value_name = "RFC3339")]
    pub backfill: Option<String>,
    /// Guard for backfill (AD28).
    #[arg(long, action = ArgAction::SetTrue)]
    pub release_backfill_confirmed: bool,

    /// Prometheus base URL.
    #[arg(long, env = "EFE_PROM_URL", default_value = "http://prometheus.monitoring.svc:9090")]
    pub prom_url: String,
    /// Override the Prom URL at runtime (PR #21 G6 mirror).
    #[arg(long, value_name = "URL")]
    pub target_override: Option<String>,

    /// Storage root for daily Parquet output.
    #[arg(long, env = "EFE_STORAGE_ROOT", default_value = "./slos")]
    pub storage_root: PathBuf,
    /// Chart version under measurement (REQUIRED — no default).
    #[arg(long, env = "EFE_CHART_VERSION")]
    pub chart_version: Option<String>,
    /// Region label (cardinality-capped allow-list).
    #[arg(long, default_value = "global")]
    pub region: String,

    /// Output format.
    #[arg(long, value_enum, default_value = "json")]
    pub format: OutputFormat,
    /// Dry-run : compute but do not write.
    #[arg(long, action = ArgAction::SetTrue)]
    pub dry_run: bool,
}

// ─────────────────────────────────────────────────────────────────
//                              rollup
// ─────────────────────────────────────────────────────────────────
#[derive(Args, Debug, Clone)]
pub struct RollupArgs {
    /// Which stage to run.
    #[arg(long, value_enum, default_value = "t1")]
    pub stage: RollupStage,
    /// Period for the stage (e.g. `2026-05-15` for T1, `2026-05` for T2,
    /// `Q2-2026` for T3).
    #[arg(long, value_name = "PERIOD")]
    pub period: String,

    #[arg(long, env = "EFE_CHART_VERSION")]
    pub chart_version: Option<String>,
    #[arg(long, default_value = "global")]
    pub region: String,
    #[arg(long, env = "EFE_STORAGE_ROOT", default_value = "./slos")]
    pub storage_root: PathBuf,

    /// For T3 : path to the cosign signing identity / key.  Phase-2
    /// expects keyless OIDC in CI — local runs may pass `--cosign-disabled`.
    #[arg(long, env = "COSIGN_IDENTITY")]
    pub cosign_identity: Option<String>,
    /// Skip signing (CI workflow uses a separate cosign step).
    #[arg(long, action = ArgAction::SetTrue)]
    pub cosign_disabled: bool,

    /// Dry-run.
    #[arg(long, action = ArgAction::SetTrue)]
    pub dry_run: bool,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollupStage {
    /// hourly Parquet — typical CronJob invocation.
    T1,
    /// monthly JSON aggregator.
    T2,
    /// quarterly signed evidence builder.
    T3,
}

// ─────────────────────────────────────────────────────────────────
//                              publish
// ─────────────────────────────────────────────────────────────────
#[derive(Args, Debug, Clone)]
pub struct PublishArgs {
    /// mdbook source directory.
    #[arg(long, default_value = "slos/site")]
    pub source: PathBuf,
    /// Output directory (gh-pages target).
    #[arg(long, default_value = "slos/site/book")]
    pub output: PathBuf,
    /// Storage root (consumes monthly + quarterly JSONs to emit
    /// `current.json` + `/history/{yyyy-mm}.json` + badges).
    #[arg(long, env = "EFE_STORAGE_ROOT", default_value = "./slos")]
    pub storage_root: PathBuf,
    #[arg(long, env = "EFE_CHART_VERSION")]
    pub chart_version: Option<String>,
}

// ─────────────────────────────────────────────────────────────────
//                              evidence
// ─────────────────────────────────────────────────────────────────
#[derive(Args, Debug, Clone)]
pub struct EvidenceArgs {
    #[command(subcommand)]
    pub action: EvidenceAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum EvidenceAction {
    /// Sign a quarterly PDF using cosign keyless OIDC.
    Sign {
        #[arg(value_name = "PDF")]
        pdf: PathBuf,
        #[arg(long, env = "COSIGN_IDENTITY")]
        identity: Option<String>,
    },
    /// Verify a signed PDF — exits 0 on valid, non-0 on tamper.
    Verify {
        #[arg(value_name = "PDF")]
        pdf: PathBuf,
        /// Path to the public key (default : `deploy/cosign/slo-evidence.pub`).
        #[arg(long, default_value = "deploy/cosign/slo-evidence.pub")]
        pubkey: PathBuf,
    },
}

// ─────────────────────────────────────────────────────────────────
//                              list
// ─────────────────────────────────────────────────────────────────
#[derive(Args, Debug, Clone)]
pub struct ListArgs {
    /// Filter to a single SLO (default = all 4).
    #[arg(long)]
    pub slo: Option<String>,
    /// Range (humantime — `7d`, `30d`, `90d`).
    #[arg(long, default_value = "30d")]
    pub range: String,
    #[arg(long, env = "EFE_STORAGE_ROOT", default_value = "./slos")]
    pub storage_root: PathBuf,
    #[arg(long, env = "EFE_CHART_VERSION")]
    pub chart_version: Option<String>,
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}

// ─────────────────────────────────────────────────────────────────
//                              validate
// ─────────────────────────────────────────────────────────────────
#[derive(Args, Debug, Clone)]
pub struct ValidateArgs {
    /// Root directory to validate (recursively).
    #[arg(value_name = "ROOT", default_value = "slos")]
    pub root: PathBuf,
}

// ─────────────────────────────────────────────────────────────────
//                       common enum types
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
pub enum OutputFormat {
    Text,
    Json,
    Csv,
}

// ─────────────────────────────────────────────────────────────────
//                       error wrapper
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

impl ComputeArgs {
    /// Cross-field validation (AD28 + AD3).
    pub fn validate(&self) -> Result<(), SloError> {
        if self.backfill.is_some() && !self.release_backfill_confirmed {
            return Err(SloError::Config(
                "backfill requires --release-backfill-confirmed (AD28)".into(),
            ));
        }
        if self.chart_version.is_none() {
            return Err(SloError::Config(
                "--chart-version is required (set via env EFE_CHART_VERSION)".into(),
            ));
        }
        // region allow-list (cardinality cap, T104 mitigation).
        const ALLOWED_REGIONS: &[&str] = &["global", "us-east-1", "us-west-2", "eu-west-1", "ap-south-1"];
        if !ALLOWED_REGIONS.contains(&self.region.as_str()) {
            return Err(SloError::Config(format!(
                "region '{}' not in allow-list {ALLOWED_REGIONS:?} (T104 mitigation)",
                self.region
            )));
        }
        Ok(())
    }
}

impl RollupArgs {
    pub fn validate(&self) -> Result<(), SloError> {
        if self.chart_version.is_none() {
            return Err(SloError::Config(
                "--chart-version is required (set via env EFE_CHART_VERSION)".into(),
            ));
        }
        if matches!(self.stage, RollupStage::T3) && !self.cosign_disabled && self.cosign_identity.is_none() {
            return Err(SloError::Config(
                "T3 requires --cosign-identity or --cosign-disabled".into(),
            ));
        }
        Ok(())
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
    fn compute_requires_chart_version() {
        let c = Cli::parse_from(["x", "compute"]);
        if let Command::Compute(a) = c.command {
            let e = a.validate().unwrap_err();
            assert_eq!(e.exit_code(), ExitCode::Config);
        }
    }

    #[test]
    fn backfill_requires_confirmation() {
        let c = Cli::parse_from([
            "x", "compute",
            "--chart-version", "0.6.0",
            "--backfill", "2026-05-15T03:00:00Z",
        ]);
        if let Command::Compute(a) = c.command {
            let e = a.validate().unwrap_err();
            assert!(matches!(e, SloError::Config(_)));
        }
    }

    #[test]
    fn region_allow_list_enforced() {
        let c = Cli::parse_from([
            "x", "compute",
            "--chart-version", "0.6.0",
            "--region", "mars-orbital-1",
        ]);
        if let Command::Compute(a) = c.command {
            let e = a.validate().unwrap_err();
            assert!(matches!(e, SloError::Config(_)));
        }
    }

    #[test]
    fn rollup_t3_requires_cosign() {
        let c = Cli::parse_from([
            "x", "rollup",
            "--stage", "t3",
            "--period", "Q2-2026",
            "--chart-version", "0.6.0",
        ]);
        if let Command::Rollup(a) = c.command {
            let e = a.validate().unwrap_err();
            assert!(matches!(e, SloError::Config(_)));
        }
    }
}
