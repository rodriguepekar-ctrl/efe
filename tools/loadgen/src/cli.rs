//! PR #20 § 7 + Appendix A — clap-based CLI.
//!
//! 40+ flags across 5 subcommands.  The full grammar is documented in
//! `docs/EFE_A1_PHASE2_PR20_ARCHITECTURE.md` § 34.
//!
//! ## Subcommands
//!
//! | Subcommand | Purpose |
//! |---|---|
//! | `run` (default) | Execute a load test |
//! | `validate`      | Dry-run config validation, no traffic |
//! | `replay`        | Replay a recorded session |
//! | `aggregate`     | Aggregate multi-pod result files |
//! | `compare`       | Compare two result files |
//! | `version`       | Print version + git_sha |
//!
//! ## Stability
//!
//! Flag names + short forms are committed in STABILITY_CONTRACT v1.2.
//! Renaming a flag is a chart-major contract break.

use std::path::PathBuf;
use std::time::Duration;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};

use crate::error::{ExitCode, LoadgenError};

/// Top-level parsed CLI structure.
#[derive(Parser, Debug, Clone)]
#[command(
    name = "efe-meta-swarm-loadgen",
    version,
    about = "Production-grade L2 meta-swarm load generator (PR #20)",
    long_about = "Production-grade, deterministic, regression-safe load generator for the EFE Phase-2 meta-swarm gRPC L2 surface. See docs/EFE_A1_PHASE2_PR20_ARCHITECTURE.md.",
    propagate_version = true
)]
pub struct Cli {
    /// Subcommand to execute.  Defaults to `run` if omitted.
    #[command(subcommand)]
    pub command: Command,

    /// Log format on stderr.
    #[arg(
        long,
        global = true,
        value_enum,
        default_value = "text",
        env = "EFE_LOADGEN_LOG_FORMAT"
    )]
    pub log_format: LogFormat,

    /// Log level filter.  `RUST_LOG` env var wins if set explicitly.
    #[arg(
        long,
        global = true,
        value_enum,
        default_value = "info",
        env = "EFE_LOADGEN_LOG_LEVEL"
    )]
    pub log_level: LogLevel,
}

impl Cli {
    /// Parse the process arguments.  Returns the parsed CLI on
    /// success ; wraps clap errors in a [`CliParseError`] so the
    /// caller can map them to the correct exit code.
    pub fn parse_args() -> Result<Self, CliParseError> {
        Self::try_parse().map_err(CliParseError::from)
    }
}

/// Subcommand dispatch surface.
#[derive(Subcommand, Debug, Clone)]
#[allow(clippy::large_enum_variant)] // RunArgs is hot path ; variants are short-lived
pub enum Command {
    /// Run a load test (default subcommand).
    Run(RunArgs),

    /// Validate the configuration without sending any traffic.
    Validate(ValidateArgs),

    /// Replay a previously recorded session against a new target.
    Replay(ReplayArgs),

    /// Aggregate result files from a multi-pod run.
    Aggregate(AggregateArgs),

    /// Compare two result files (baseline vs candidate).
    Compare(CompareArgs),

    /// Print version, git_sha, rustc_version, build_date.
    Version,
}

// ────────────────────────────────────────────────────────────────
//                       run / validate / replay
// ────────────────────────────────────────────────────────────────

/// Arguments for the `run` subcommand — the heavy hitter.  Full grammar
/// is documented in architecture § 34 Appendix A.
#[derive(Args, Debug, Clone)]
pub struct RunArgs {
    // ── Targeting ───────────────────────────────────────────────
    /// gRPC endpoint URL.  Repeatable for multi-target runs ;
    /// optionally suffix `@<weight>` for weighted random selection
    /// (e.g., `grpc://ctrl-1:50051@0.5`).
    #[arg(long, value_name = "URL", env = "EFE_LOADGEN_TARGET")]
    pub target: Vec<String>,

    /// Role of the targeted endpoints.
    #[arg(long, value_enum, default_value = "controller")]
    pub target_role: TargetRole,

    /// HTTP/2 connections per target.
    #[arg(long, default_value_t = 4)]
    pub conns_per_target: u32,

    /// Per-request gRPC deadline (e.g., `3s`, `500ms`).
    #[arg(long, value_parser = parse_duration)]
    pub per_request_deadline: Option<Duration>,

    // ── Workload / profile ──────────────────────────────────────
    /// Path to a workload profile YAML.  CLI flags override profile
    /// values per the precedence rules in architecture § 8.
    #[arg(long, value_name = "PATH")]
    pub profile: Option<PathBuf>,

    /// Traffic pattern.
    #[arg(long, value_enum, default_value = "constant")]
    pub pattern: PatternKind,

    /// Workload shape (single).  Use `--workload-mix` for blended.
    #[arg(long, value_enum, default_value = "iterate")]
    pub workload: WorkloadKind,

    /// Comma-separated `verb=weight` pairs for mixed workloads,
    /// e.g., `iterate=0.95,state=0.05`.  Weights must sum to ~1.0.
    #[arg(long, value_name = "K=V,...")]
    pub workload_mix: Option<String>,

    // ── Rate control ────────────────────────────────────────────
    /// Target QPS.  Default 10.  Hard-capped to 1 000 unless
    /// `--i-am-sure` ; absolute ceiling 100 000.  Mutually exclusive
    /// with `--concurrency`.
    #[arg(long, default_value_t = 10.0)]
    pub qps: f64,

    /// Closed-loop concurrency.  Mutually exclusive with `--qps`
    /// (clap's `conflicts_with` enforces this).
    #[arg(long, conflicts_with = "qps")]
    pub concurrency: Option<u32>,

    /// Rate control mode.
    #[arg(long, value_enum, default_value = "hybrid")]
    pub rate_mode: RateMode,

    /// Linear ramp-up duration.
    #[arg(long, default_value = "30s", value_parser = parse_duration)]
    pub ramp_up: Duration,

    // ── Termination ─────────────────────────────────────────────
    /// Maximum wall-clock duration (e.g., `5m`, `1h`).
    #[arg(long, value_parser = parse_duration)]
    pub duration: Option<Duration>,

    /// Maximum total requests.
    #[arg(long)]
    pub requests: Option<u64>,

    /// Run until SLO breach.  Repeatable.  Format : `slo_id=target`,
    /// e.g., `iterate-availability=0.999`.
    #[arg(long = "until-slo", value_name = "SLO=VAL")]
    pub until_slo: Vec<String>,

    /// Graceful drain deadline on SIGINT/SIGTERM.
    #[arg(long, default_value = "5s", value_parser = parse_duration)]
    pub shutdown_deadline: Duration,

    // ── mTLS ────────────────────────────────────────────────────
    /// Client certificate (PEM, chain).
    #[arg(long, env = "EFE_LOADGEN_TLS_CERT")]
    pub tls_cert: Option<PathBuf>,

    /// Client private key (PEM, PKCS-8 ECDSA-P256 preferred per PR #16).
    #[arg(long, env = "EFE_LOADGEN_TLS_KEY")]
    pub tls_key: Option<PathBuf>,

    /// CA bundle for server verification (PEM).
    #[arg(long, env = "EFE_LOADGEN_TLS_CA")]
    pub tls_ca: Option<PathBuf>,

    /// Expected server SAN (DNS or URI form per PR #16).
    #[arg(long)]
    pub expected_san: Option<String>,

    /// Disable cert verification — DEV ONLY.  Refuses to set with
    /// `--mode=production`.
    #[arg(long, action = ArgAction::SetTrue)]
    pub tls_insecure: bool,

    // ── Observability ───────────────────────────────────────────
    /// Self-metrics listen address.
    #[arg(long, default_value = "0.0.0.0:9095", env = "EFE_LOADGEN_PROM_LISTEN")]
    pub prom_listen: String,

    /// OTLP/gRPC collector endpoint.  When unset, OTel is disabled.
    #[arg(long, env = "EFE_LOADGEN_OTEL_ENDPOINT")]
    pub otel_endpoint: Option<String>,

    /// Trace sampling rate.  Default 1.0 — every loadgen request gets
    /// a span.  The collector-side tail sampler decides retention.
    #[arg(long, default_value_t = 1.0)]
    pub otel_sampler: f64,

    /// OTel `service.name` resource attribute.
    #[arg(long, default_value = "efe-meta-swarm-loadgen")]
    pub otel_service_name: String,

    // ── Output ──────────────────────────────────────────────────
    /// Output format for the result file + stdout.
    #[arg(long, value_enum, default_value = "ndjson")]
    pub output_format: OutputFormat,

    /// Result file path (omit for stdout).
    #[arg(long, value_name = "PATH")]
    pub output_file: Option<PathBuf>,

    /// Record every request to the result file in replay-ready format.
    #[arg(long, action = ArgAction::SetTrue)]
    pub record: bool,

    // ── Determinism ─────────────────────────────────────────────
    /// Deterministic seed.  Default = wall-clock nanos.  Same seed +
    /// same profile + same target list = bit-identical wire bytes.
    #[arg(long)]
    pub seed: Option<u64>,

    // ── Safety knobs ────────────────────────────────────────────
    /// Run mode.  `production` requires `--confirm-prod`.
    #[arg(long, value_enum, default_value = "dev", env = "EFE_LOADGEN_MODE")]
    pub mode: RunMode,

    /// Required when `--mode=production`.
    #[arg(long, action = ArgAction::SetTrue)]
    pub confirm_prod: bool,

    /// Validate config + exit ; do not send traffic.
    #[arg(long, action = ArgAction::SetTrue)]
    pub dry_run: bool,

    /// Enable Step + Reset (mutating) verbs.  Refused in production.
    #[arg(long, action = ArgAction::SetTrue)]
    pub allow_mutation: bool,

    /// Unlock `--qps` > 1000 (up to the hard cap of 100 000).
    #[arg(long, action = ArgAction::SetTrue)]
    pub i_am_sure: bool,

    // ── Chaos / coordination ────────────────────────────────────
    /// Tag every span with this fault marker.  Consumed by chaos
    /// benches (PR #21) to filter traces by injected fault.
    #[arg(long, value_name = "LABEL")]
    pub chaos_marker: Option<String>,

    /// Block startup until the given file exists.  Used by chaos
    /// benches to coordinate the loadgen start with the fault
    /// injection start.
    #[arg(long, value_name = "PATH")]
    pub wait_for_signal: Option<PathBuf>,
}

/// Arguments for the `validate` subcommand.
#[derive(Args, Debug, Clone)]
pub struct ValidateArgs {
    /// Profile YAML to validate.
    #[arg(long, value_name = "PATH")]
    pub profile: PathBuf,
}

/// Arguments for the `replay` subcommand.
#[derive(Args, Debug, Clone)]
pub struct ReplayArgs {
    /// Recorded session file (CSV / ndjson / parquet).
    #[arg(long, value_name = "PATH")]
    pub replay_file: PathBuf,

    /// Replay mode.
    #[arg(long, value_enum, default_value = "preserve-spacing")]
    pub replay_mode: ReplayMode,

    /// Speed multiplier (for `accelerate` mode).
    #[arg(long, default_value_t = 1.0)]
    pub replay_speed: f64,

    /// Target endpoint(s) for replay.
    #[arg(long, value_name = "URL")]
    pub target: Vec<String>,

    /// mTLS cert (same shape as the `run` subcommand).
    #[arg(long)]
    pub tls_cert: Option<PathBuf>,
    #[arg(long)]
    pub tls_key: Option<PathBuf>,
    #[arg(long)]
    pub tls_ca: Option<PathBuf>,
    #[arg(long)]
    pub expected_san: Option<String>,
}

/// Arguments for the `aggregate` subcommand.
#[derive(Args, Debug, Clone)]
pub struct AggregateArgs {
    /// Glob pattern for input result files.
    #[arg(long, value_name = "GLOB")]
    pub pattern: String,

    /// Output file (aggregated parquet).
    #[arg(long, value_name = "PATH")]
    pub output: PathBuf,

    /// Emit a JSON SLO report alongside the aggregate file.
    #[arg(long, action = ArgAction::SetTrue)]
    pub emit_slo_report: bool,

    /// Report format.
    #[arg(long, value_enum, default_value = "json")]
    pub output_format: ReportFormat,
}

/// Arguments for the `compare` subcommand.
#[derive(Args, Debug, Clone)]
pub struct CompareArgs {
    /// Baseline result file.
    #[arg(long, value_name = "PATH")]
    pub baseline: PathBuf,

    /// Candidate result file.
    #[arg(long, value_name = "PATH")]
    pub candidate: PathBuf,

    /// Metric to compare.
    #[arg(long, value_enum, default_value = "p95-ms")]
    pub metric: CompareMetric,

    /// Acceptable regression percentage (e.g., 10 = 10%).
    #[arg(long, default_value_t = 10.0)]
    pub tolerance: f64,
}

// ────────────────────────────────────────────────────────────────
//                          Enum types
// ────────────────────────────────────────────────────────────────

/// Log output format.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    /// Human-readable text on stderr.
    Text,
    /// Structured JSON-per-line on stderr.
    Json,
}

/// Log filter level.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// Per-event tracing.
    Trace,
    /// Debug-level diagnostics.
    Debug,
    /// Operator-visible status (default).
    Info,
    /// Warnings only.
    Warn,
    /// Errors only.
    Error,
}

impl LogLevel {
    /// `tracing_subscriber` filter string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info  => "info",
            Self::Warn  => "warn",
            Self::Error => "error",
        }
    }
}

/// Endpoint role.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetRole {
    /// Meta-swarm controller (default).
    Controller,
    /// Direct participant.
    Participant,
}

/// Traffic pattern catalogue.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternKind {
    /// Fixed QPS for the whole run.
    Constant,
    /// Linear / exponential ramp.
    Ramp,
    /// Bursty (baseline + periodic spikes).
    Burst,
    /// Sinusoidal day/night cycle.
    Diurnal,
    /// From a recorded session file.
    Replay,
}

/// Workload shape catalogue.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadKind {
    /// Iterate-heavy (default).
    Iterate,
    /// Direct participant Step (bypass controller).
    Step,
    /// Reset + State torture.
    Reset,
    /// State queries only.
    State,
    /// Weighted blend per `--workload-mix`.
    Mixed,
}

/// Rate controller strategy.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateMode {
    /// Open-loop token bucket.
    Open,
    /// Closed-loop fixed concurrency.
    Closed,
    /// Open with concurrency cap (default).
    Hybrid,
}

/// Output / record format.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    /// Live TUI (requires TTY ; falls back to ndjson otherwise).
    Terminal,
    /// JSON summary on stdout at end of run.
    Json,
    /// CSV per-request.
    Csv,
    /// Newline-delimited JSON per-request.
    Ndjson,
    /// Apache Parquet (feature-gated).
    Parquet,
}

/// Run mode (safety bracket).
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    /// Local development (default).
    Dev,
    /// Pre-production cluster.
    Staging,
    /// Production — requires `--confirm-prod`.
    Production,
}

impl RunMode {
    /// True when this mode disables certain safety guards.
    #[must_use]
    pub const fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }
}

/// Replay timing strategy.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayMode {
    /// Preserve wall-clock spacing (1× replay).
    PreserveSpacing,
    /// Time-compress the timeline.
    Accelerate,
    /// Flood at maximum rate (ignore spacing).
    Drain,
}

/// SLO report format.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportFormat {
    Json,
    Yaml,
    Text,
}

/// Metric to compare in the `compare` subcommand.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareMetric {
    P50Ms,
    P95Ms,
    P99Ms,
    ErrorRate,
    Throughput,
}

// ────────────────────────────────────────────────────────────────
//                  CLI error wrapper + parsers
// ────────────────────────────────────────────────────────────────

/// Wrapper for clap errors that exposes the right exit code.
#[derive(Debug)]
pub struct CliParseError(pub clap::Error);

impl CliParseError {
    /// Map clap error to the loadgen exit code.
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        use clap::error::ErrorKind;
        match self.0.kind() {
            // --help and --version are not "errors" — exit 0.
            ErrorKind::DisplayHelp
            | ErrorKind::DisplayVersion
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => ExitCode::Success,
            // Anything else = invalid config.
            _ => ExitCode::Config,
        }
    }
}

impl From<clap::Error> for CliParseError {
    fn from(e: clap::Error) -> Self {
        let _ = e.print(); // best-effort; clap prints to the right stream
        Self(e)
    }
}

/// `humantime`-backed duration parser usable from `value_parser`.
fn parse_duration(s: &str) -> Result<Duration, String> {
    humantime::parse_duration(s).map_err(|e| format!("invalid duration : {e}"))
}

// ────────────────────────────────────────────────────────────────
//                    Cross-field validation
// ────────────────────────────────────────────────────────────────

impl RunArgs {
    /// Validate cross-field invariants after clap parsing.  Returns a
    /// [`LoadgenError`] with the right exit code, ready for the engine.
    pub fn validate(&self) -> Result<(), LoadgenError> {
        // AD18 — production mode requires --confirm-prod.
        if self.mode.is_production() && !self.confirm_prod {
            return Err(LoadgenError::ProductionConfirmationMissing);
        }
        // AD18 — --tls-insecure refused in production.
        if self.mode.is_production() && self.tls_insecure {
            return Err(LoadgenError::FlagConflict(
                "--tls-insecure cannot be used with --mode=production".into(),
            ));
        }
        // AD5 — QPS hard cap.
        let cap = if self.i_am_sure {
            crate::QPS_HARD_CAP
        } else {
            crate::QPS_SAFE_DEFAULT_CAP
        };
        if self.qps.is_finite() && self.qps > cap {
            return Err(LoadgenError::QpsCapExceeded { requested: self.qps, cap });
        }
        if !self.qps.is_finite() || self.qps < 0.0 {
            return Err(LoadgenError::InvalidConfig(format!(
                "--qps must be a finite non-negative number (got {})",
                self.qps
            )));
        }
        // AD40 — mutating verbs refused in production unless overridden via profile.
        if self.mode.is_production() && self.allow_mutation {
            return Err(LoadgenError::FlagConflict(
                "--allow-mutation cannot be used with --mode=production".into(),
            ));
        }
        // AD3 — at least one --target unless --profile carries them.
        if self.target.is_empty() && self.profile.is_none() {
            return Err(LoadgenError::InvalidConfig(
                "at least one --target or --profile is required".into(),
            ));
        }
        // mTLS cert triplet : all three present or none.
        let cert_count = [
            self.tls_cert.is_some(),
            self.tls_key.is_some(),
            self.tls_ca.is_some(),
        ]
        .iter()
        .filter(|x| **x)
        .count();
        if cert_count != 0 && cert_count != 3 {
            return Err(LoadgenError::FlagConflict(
                "mTLS requires all three : --tls-cert, --tls-key, --tls-ca".into(),
            ));
        }
        Ok(())
    }
}

// ────────────────────────────────────────────────────────────────
//                              Tests
// ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_compiles_via_command_factory() {
        Cli::command().debug_assert();
    }

    #[test]
    fn run_args_default_qps_is_safe() {
        let args = ["run", "--target", "grpc://localhost:50051"];
        let cli = Cli::parse_from(["loadgen"].iter().chain(args.iter()));
        if let Command::Run(r) = cli.command {
            assert_eq!(r.qps, 10.0);
            assert!(!r.confirm_prod);
            assert!(!r.tls_insecure);
            assert_eq!(r.rate_mode, RateMode::Hybrid);
            assert_eq!(r.pattern, PatternKind::Constant);
        } else {
            panic!("expected Run subcommand");
        }
    }

    #[test]
    fn production_without_confirm_rejected() {
        let cli = Cli::parse_from([
            "loadgen", "run", "--target", "grpc://prod.svc:50051",
            "--mode", "production",
        ]);
        if let Command::Run(r) = cli.command {
            let err = r.validate().unwrap_err();
            assert_eq!(err.exit_code(), ExitCode::Config);
            assert!(matches!(err, LoadgenError::ProductionConfirmationMissing));
        }
    }

    #[test]
    fn qps_above_safe_cap_without_i_am_sure_rejected() {
        let cli = Cli::parse_from([
            "loadgen", "run", "--target", "grpc://x:50051",
            "--qps", "5000",
        ]);
        if let Command::Run(r) = cli.command {
            let err = r.validate().unwrap_err();
            assert_eq!(err.exit_code(), ExitCode::Config);
            assert!(matches!(err, LoadgenError::QpsCapExceeded { .. }));
        }
    }

    #[test]
    fn qps_above_safe_cap_with_i_am_sure_accepted() {
        let cli = Cli::parse_from([
            "loadgen", "run", "--target", "grpc://x:50051",
            "--qps", "5000", "--i-am-sure",
        ]);
        if let Command::Run(r) = cli.command {
            r.validate().expect("should accept under hard cap");
        }
    }

    #[test]
    fn qps_above_hard_cap_always_rejected() {
        let cli = Cli::parse_from([
            "loadgen", "run", "--target", "grpc://x:50051",
            "--qps", "200000", "--i-am-sure",
        ]);
        if let Command::Run(r) = cli.command {
            let err = r.validate().unwrap_err();
            assert!(matches!(err, LoadgenError::QpsCapExceeded { .. }));
        }
    }

    #[test]
    fn tls_triplet_must_be_complete() {
        let cli = Cli::parse_from([
            "loadgen", "run", "--target", "grpc://x:50051",
            "--tls-cert", "/etc/tls/tls.crt",
        ]);
        if let Command::Run(r) = cli.command {
            let err = r.validate().unwrap_err();
            assert!(matches!(err, LoadgenError::FlagConflict(_)));
        }
    }

    #[test]
    fn tls_insecure_refused_in_production() {
        let cli = Cli::parse_from([
            "loadgen", "run", "--target", "grpc://x:50051",
            "--mode", "production", "--confirm-prod", "--tls-insecure",
        ]);
        if let Command::Run(r) = cli.command {
            let err = r.validate().unwrap_err();
            assert!(matches!(err, LoadgenError::FlagConflict(_)));
        }
    }

    #[test]
    fn allow_mutation_refused_in_production() {
        let cli = Cli::parse_from([
            "loadgen", "run", "--target", "grpc://x:50051",
            "--mode", "production", "--confirm-prod", "--allow-mutation",
        ]);
        if let Command::Run(r) = cli.command {
            let err = r.validate().unwrap_err();
            assert!(matches!(err, LoadgenError::FlagConflict(_)));
        }
    }

    #[test]
    fn qps_negative_rejected() {
        // Use `=` form so clap doesn't treat `-1` as a short flag.
        let cli = Cli::parse_from([
            "loadgen", "run", "--target", "grpc://x:50051",
            "--qps=-1",
        ]);
        if let Command::Run(r) = cli.command {
            let err = r.validate().unwrap_err();
            assert!(matches!(err, LoadgenError::InvalidConfig(_)));
        }
    }

    #[test]
    fn qps_concurrency_mutually_exclusive() {
        // clap's `conflicts_with` should reject at parse time.
        let result = Cli::try_parse_from([
            "loadgen", "run", "--target", "grpc://x:50051",
            "--qps", "10", "--concurrency", "50",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn ramp_up_default_is_30s() {
        let cli = Cli::parse_from([
            "loadgen", "run", "--target", "grpc://x:50051",
        ]);
        if let Command::Run(r) = cli.command {
            assert_eq!(r.ramp_up, Duration::from_secs(30));
        }
    }

    #[test]
    fn output_format_default_is_ndjson() {
        let cli = Cli::parse_from(["loadgen", "run", "--target", "grpc://x:50051"]);
        if let Command::Run(r) = cli.command {
            assert_eq!(r.output_format, OutputFormat::Ndjson);
        }
    }

    #[test]
    fn run_mode_enum_production_detection() {
        assert!(RunMode::Production.is_production());
        assert!(!RunMode::Staging.is_production());
        assert!(!RunMode::Dev.is_production());
    }

    #[test]
    fn log_level_strings_are_stable() {
        // STABILITY_CONTRACT — these strings flow into env filters.
        assert_eq!(LogLevel::Info.as_str(), "info");
        assert_eq!(LogLevel::Error.as_str(), "error");
        assert_eq!(LogLevel::Debug.as_str(), "debug");
    }
}
