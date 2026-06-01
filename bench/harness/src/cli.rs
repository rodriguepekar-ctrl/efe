//! PR #21 § 9.2 + Appendix A — clap CLI.

use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};

use crate::error::{BenchError, ExitCode};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "efe-bench-harness",
    version,
    about = "Public-bench harness for the EFE meta-swarm L2 surface (PR #21)",
    long_about = "Reproducible cross-version benchmark + chaos orchestrator. Consumes PR #20 loadgen + chaos-mesh CRDs. See docs/EFE_A1_PHASE2_PR21_ARCHITECTURE.md."
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

#[derive(Subcommand, Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Command {
    /// Execute one or more scenarios end-to-end.
    Run(RunArgs),
    /// List discovered scenarios.
    List(ListArgs),
    /// Validate scenario YAMLs against the schema.
    Validate(ValidateArgs),
    /// Compare a result file vs a baseline file.
    Compare(CompareArgs),
    /// Regenerate baselines (release-tag-only, AD16).
    BaselineRegen(BaselineRegenArgs),
    /// Build GitHub Pages site + badges (PR #21 + PR #24).
    Publish(PublishArgs),
    /// PR #24 § 12 — generate signed quarterly compliance evidence.
    PublishQuarterly(PublishQuarterlyArgs),
    /// PR #24 § 12 + AD26 — cross-chain verifier (accepts both PR #22
    /// SLO + PR #24 bench quarterly PDFs).
    Evidence(EvidenceArgs),
    /// Print version + git_sha + rustc_version.
    Version,
}

#[derive(Args, Debug, Clone)]
pub struct EvidenceArgs {
    #[command(subcommand)]
    pub action: EvidenceAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum EvidenceAction {
    /// Verify a signed PDF (embedded SHA + optional cosign bundle).
    Verify {
        /// Path to the PDF to verify.
        #[arg(value_name = "PDF")]
        pdf: PathBuf,
    },
}

// ────────────────────────────────────────────────────────────────
//                            run
// ────────────────────────────────────────────────────────────────
#[derive(Args, Debug, Clone)]
pub struct RunArgs {
    /// Scenario IDs to run (e.g. `S01_baseline_100qps_5min`).
    /// Repeatable.  If empty AND `--all` is set, runs every discovered
    /// scenario.
    #[arg(long, value_name = "ID")]
    pub scenario: Vec<String>,

    /// Run every discovered scenario.
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "scenario")]
    pub all: bool,

    /// Directory containing scenario YAML files.
    #[arg(long, default_value = "bench/scenarios")]
    pub scenarios: PathBuf,

    /// Baselines directory (versioned by chart minor).
    #[arg(long, default_value = "bench_baselines")]
    pub baselines: PathBuf,

    /// Output directory for per-run artefacts.
    #[arg(long, value_name = "PATH")]
    pub output_dir: Option<PathBuf>,

    /// Path to a kubeconfig (when `--provisioner=external`).
    #[arg(long, env = "KUBECONFIG")]
    pub kubeconfig: Option<PathBuf>,

    /// Cluster provisioner.
    #[arg(long, value_enum, default_value = "external")]
    pub provisioner: ProvisionerKind,

    /// Fresh cluster per scenario (slow, hermetic).
    #[arg(long, action = ArgAction::SetTrue)]
    pub cluster_per_scenario: bool,

    /// Spawn loadgen as a subprocess instead of in-process library.
    #[arg(long, action = ArgAction::SetTrue)]
    pub loadgen_subprocess: bool,

    /// Skip cluster teardown (debug).
    #[arg(long, action = ArgAction::SetTrue)]
    pub skip_teardown: bool,

    /// Write the PR comment markdown to this file.
    #[arg(long, value_name = "PATH")]
    pub pr_comment_file: Option<PathBuf>,

    /// Write the JSON verdict to this file (CI consumption).
    #[arg(long, value_name = "PATH")]
    pub verdict_json_file: Option<PathBuf>,

    /// Chart version under test (default : detect from helm).
    #[arg(long)]
    pub chart_version: Option<String>,

    /// Image tag (default : detect from running pods).
    #[arg(long)]
    pub image_tag: Option<String>,

    /// Deterministic loadgen seed (default = wall-clock nanos).
    #[arg(long)]
    pub seed: Option<u64>,

    /// Run mode.  `production` is **REFUSED** (AD11, T86).
    #[arg(long, value_enum, default_value = "dev")]
    pub mode: BenchMode,

    /// Dry-run : validate scenarios, no cluster/load/chaos.
    #[arg(long, action = ArgAction::SetTrue)]
    pub dry_run: bool,

    /// Override the loadgen target endpoint URL at runtime.
    ///
    /// When set, every scenario's `loadgen.target.endpoints[].url` is
    /// rewritten to this URL.  Weight + role + per_request_deadline are
    /// preserved.  This is the recommended escape hatch when running
    /// the harness from a CI runner that cannot resolve in-cluster DNS
    /// (PR #21 § 14 — G1/G2/G6 closure).
    ///
    /// Example : `--target-override grpc://localhost:50051`
    #[arg(long, value_name = "URL")]
    pub target_override: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ListArgs {
    #[arg(long, default_value = "bench/scenarios")]
    pub scenarios: PathBuf,
    /// `text` (default), `json` or `yaml`.
    #[arg(long, value_enum, default_value = "text")]
    pub format: ListFormat,
}

#[derive(Args, Debug, Clone)]
pub struct ValidateArgs {
    #[arg(long, default_value = "bench/scenarios")]
    pub scenarios: PathBuf,
}

#[derive(Args, Debug, Clone)]
pub struct CompareArgs {
    #[arg(long, value_name = "PATH")]
    pub result: PathBuf,
    #[arg(long, value_name = "PATH")]
    pub baseline: PathBuf,
    #[arg(long, value_enum, default_value = "markdown")]
    pub format: CompareFormat,
}

#[derive(Args, Debug, Clone)]
pub struct BaselineRegenArgs {
    /// Chart version tag (e.g. `v0.6.0`).
    #[arg(long, value_name = "TAG")]
    pub chart_version: String,
    #[arg(long, default_value = "bench/scenarios")]
    pub scenarios: PathBuf,
    #[arg(long, default_value = "bench_baselines")]
    pub baselines: PathBuf,
    /// Refuses to overwrite without `--release-tag-confirmed` (AD16/AD40).
    #[arg(long, action = ArgAction::SetTrue)]
    pub release_tag_confirmed: bool,
}

#[derive(Args, Debug, Clone)]
pub struct PublishArgs {
    /// Output directory for the static site (PR #21).
    #[arg(long, default_value = "docs/bench/site/book")]
    pub output: PathBuf,
    /// Directory containing the mdbook source.
    /// **D7 fix** — default updated to `bench/publish/src` (PR #24 §
    /// 16.1).  PR #21 callers passing `--source docs/bench/site` still
    /// work (explicit override).
    #[arg(long, default_value = "bench/publish")]
    pub source: PathBuf,

    // ── PR #24 § 16 — 7 new frozen flags (STABILITY v1.6 § 24.5) ──

    /// Directory of `bench/results/{run_id}/{scenario_id}/summary.json`
    /// artefacts produced by PR #21 bench runs.
    #[arg(long, value_name = "PATH")]
    pub bench_results: Option<PathBuf>,

    /// Directory of `bench_baselines/v*/L2/*.json` produced by
    /// `efe-bench-harness baseline-regen`.
    #[arg(long, value_name = "PATH")]
    pub bench_baselines: Option<PathBuf>,

    /// Chart version under publish (default = detect from latest baseline).
    #[arg(long, env = "EFE_CHART_VERSION")]
    pub chart_version: Option<String>,

    /// AD34 — embed PR #22 SLO snapshot in `/api/current.json` (default off).
    #[arg(long, action = ArgAction::SetTrue)]
    pub embed_slos: bool,

    /// AD35 — embed PR #23 smoke badge in overview header.
    /// **D-G fix** : default is `true` ; pass `--embed-smoke false` to
    /// disable (BoolishValueParser accepts true/false/yes/no/0/1).
    #[arg(
        long,
        default_value_t = true,
        value_parser = clap::builder::BoolishValueParser::new(),
        num_args = 0..=1,
        default_missing_value = "true",
    )]
    pub embed_smoke: bool,

    /// AD38 — write to a tempdir + verify layout (no gh-pages push).
    #[arg(long, action = ArgAction::SetTrue)]
    pub dry_run: bool,

    /// AD11 — number of days of history to expose (default = 30).
    #[arg(long, default_value_t = 30)]
    pub history_window_days: u32,
}

#[derive(Args, Debug, Clone)]
pub struct PublishQuarterlyArgs {
    /// Quarter period (e.g. `Q2-2026`).
    #[arg(long, value_name = "QUARTER")]
    pub period: String,

    /// Chart version (required).
    #[arg(long, env = "EFE_CHART_VERSION")]
    pub chart_version: Option<String>,

    /// gh-pages target directory.
    #[arg(long, value_name = "PATH")]
    pub output: PathBuf,

    /// OIDC issuer for cosign keyless (default = ambient GH OIDC).
    #[arg(long, env = "COSIGN_IDENTITY")]
    pub cosign_identity: Option<String>,

    /// Skip cosign signing (local dev).  CI uses keyless OIDC.
    #[arg(long, action = ArgAction::SetTrue)]
    pub cosign_disabled: bool,

    /// Dry-run : no PDF write, no signing.
    #[arg(long, action = ArgAction::SetTrue)]
    pub dry_run: bool,
}

// ────────────────────────────────────────────────────────────────
//                         enum types
// ────────────────────────────────────────────────────────────────
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat { Text, Json }

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel { Trace, Debug, Info, Warn, Error }

impl LogLevel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace", Self::Debug => "debug",
            Self::Info => "info", Self::Warn => "warn", Self::Error => "error",
        }
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionerKind {
    Kind,
    K3s,
    External,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchMode { Dev, Staging }

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListFormat { Text, Json, Yaml }

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareFormat { Markdown, Json, Text }

// ────────────────────────────────────────────────────────────────
//                       error wrappers
// ────────────────────────────────────────────────────────────────
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
    pub fn validate(&self) -> Result<(), BenchError> {
        if self.provisioner == ProvisionerKind::External && self.kubeconfig.is_none() {
            return Err(BenchError::InvalidScenario(
                "--provisioner=external requires --kubeconfig".into(),
            ));
        }
        if !self.all && self.scenario.is_empty() {
            return Err(BenchError::InvalidScenario(
                "either --scenario <ID>... or --all is required".into(),
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
    fn run_external_requires_kubeconfig() {
        let cli = Cli::parse_from([
            "h", "run", "--scenario", "S01_baseline_100qps_5min",
            "--provisioner", "external",
        ]);
        if let Command::Run(r) = cli.command {
            let e = r.validate().unwrap_err();
            assert_eq!(e.exit_code(), ExitCode::Config);
        }
    }

    #[test]
    fn run_no_scenario_no_all_rejected() {
        let cli = Cli::parse_from(["h", "run", "--kubeconfig", "/tmp/kc"]);
        if let Command::Run(r) = cli.command {
            let e = r.validate().unwrap_err();
            assert!(matches!(e, BenchError::InvalidScenario(_)));
        }
    }

    #[test]
    fn provisioner_default_is_external() {
        let cli = Cli::parse_from([
            "h", "run", "--scenario", "S01_baseline_100qps_5min",
            "--kubeconfig", "/tmp/kc",
        ]);
        if let Command::Run(r) = cli.command {
            assert_eq!(r.provisioner, ProvisionerKind::External);
        }
    }
}
