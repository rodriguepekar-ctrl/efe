//! PR #20 § 8 — `LoadgenConfig` : the merged, validated configuration
//! passed to the engine.
//!
//! ## Precedence
//!
//! ```text
//! CLI flags  >  profile YAML  >  env vars  >  built-in defaults
//! ```
//!
//! ## Shape
//!
//! `LoadgenConfig` is the *single source of truth* for the runtime.
//! Every other module receives it (or a sub-slice) instead of reading
//! CLI args directly — keeps the engine testable in isolation.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::cli::{
    OutputFormat, PatternKind, RateMode, RunArgs, RunMode, TargetRole, WorkloadKind,
};
use crate::error::{LoadgenError, LoadgenResult};
use crate::profile::Profile;

/// A single target endpoint with an associated weight.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TargetSpec {
    /// gRPC URL.  Accepted schemes : `grpc://` (plaintext), `grpcs://`
    /// (TLS), or bare `host:port` (treated as `grpc://`).
    pub url: String,

    /// Selection weight.  Sum across targets need not equal 1 — the
    /// engine normalises.
    pub weight: f64,
}

impl Default for TargetSpec {
    fn default() -> Self {
        Self { url: "grpc://localhost:50051".into(), weight: 1.0 }
    }
}

/// mTLS material — file paths kept (no in-memory secret cache).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Client cert (PEM, chain).
    pub cert_path: PathBuf,
    /// Client private key (PEM, PKCS-8).
    pub key_path: PathBuf,
    /// CA bundle (PEM).
    pub ca_path: PathBuf,
    /// Expected server SAN.
    pub expected_san: Option<String>,
    /// Disable verification — DEV ONLY.  Rejected in production.
    pub insecure: bool,
}

/// Workload-mix : the actual proportions after CLI/profile merge.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkloadMix {
    pub iterate: f64,
    pub step: f64,
    pub reset: f64,
    pub state: f64,
}

impl WorkloadMix {
    /// Single-verb mix (one weight = 1.0, others = 0.0).
    #[must_use]
    pub fn single(verb: WorkloadKind) -> Self {
        let mut m = Self::default();
        match verb {
            WorkloadKind::Iterate | WorkloadKind::Mixed => m.iterate = 1.0,
            WorkloadKind::Step => m.step = 1.0,
            WorkloadKind::Reset => m.reset = 1.0,
            WorkloadKind::State => m.state = 1.0,
        }
        m
    }

    /// True if the weights sum to roughly 1.0 (tolerance 1e-3).
    #[must_use]
    pub fn is_normalised(&self) -> bool {
        let sum = self.iterate + self.step + self.reset + self.state;
        (sum - 1.0).abs() < 1.0e-3
    }
}

/// Pattern-specific parameters.  One variant per [`PatternKind`].
///
/// Duration fields are humantime-serialized (e.g. `500ms`, `1h`) so
/// profile YAMLs stay operator-readable.  serde's default
/// `Duration` deserializer expects `{secs, nanos}` — without the
/// `#[serde(with = "humantime_dur")]` attribute, profile loading
/// rejects every duration string with `invalid type: string`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PatternSpec {
    Constant {
        qps: f64,
    },
    Ramp {
        initial_qps: f64,
        final_qps: f64,
        #[serde(default)]
        shape: RampShape,
        #[serde(with = "humantime_dur")]
        duration: Duration,
    },
    Burst {
        baseline_qps: f64,
        burst_qps: f64,
        #[serde(with = "humantime_dur")]
        burst_duration: Duration,
        #[serde(with = "humantime_dur")]
        burst_interval: Duration,
    },
    Diurnal {
        daily_min_qps: f64,
        daily_max_qps: f64,
        #[serde(with = "humantime_dur")]
        period: Duration,
        #[serde(with = "humantime_dur", default)]
        phase: Duration,
    },
    Replay {
        file: PathBuf,
        #[serde(default)]
        mode: ReplayPlaybackMode,
        #[serde(default = "default_replay_speed")]
        speed: f64,
    },
}

fn default_replay_speed() -> f64 {
    1.0
}

/// Serde adapter for `std::time::Duration` ↔ humantime string.
mod humantime_dur {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&humantime::format_duration(*d).to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let s = String::deserialize(d)?;
        humantime::parse_duration(&s).map_err(serde::de::Error::custom)
    }
}

/// Ramp shape for [`PatternSpec::Ramp`].
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RampShape {
    #[default]
    Linear,
    Exponential,
}

/// Replay timing.  Mirrored from `crate::cli::ReplayMode` for profile
/// YAML use.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplayPlaybackMode {
    #[default]
    PreserveSpacing,
    Accelerate,
    Drain,
}

/// Rate controller parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RateSpec {
    pub mode: RateMode,
    /// For hybrid/closed modes.
    pub max_concurrency: u32,
    /// Linear ramp-up duration applied to the *effective* QPS.
    pub ramp_up: Duration,
    /// Overflow policy for open-loop mode.
    pub overflow_policy: OverflowPolicy,
}

impl Default for RateSpec {
    fn default() -> Self {
        Self {
            mode: RateMode::Hybrid,
            max_concurrency: 200,
            ramp_up: Duration::from_secs(30),
            overflow_policy: OverflowPolicy::Drop,
        }
    }
}

/// Overflow handling for open-loop mode.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverflowPolicy {
    /// Drop the slot when the bucket is empty.
    #[default]
    Drop,
    /// Block (await refill).
    Block,
}

/// Composed termination configuration — any policy can fire.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TerminationSpec {
    pub duration: Option<Duration>,
    pub requests: Option<u64>,
    pub until_slo: Vec<SloThreshold>,
    pub convergence: Option<ConvergenceCriteria>,
    pub shutdown_deadline: Duration,
}

/// SLO threshold spec ; the loadgen aborts as soon as `actual` violates
/// `target` by direction.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SloThreshold {
    /// SLO id (matches STABILITY_CONTRACT v1.1 — `iterate-availability`,
    /// `iterate-latency-p95`, etc.).
    pub slo_id: String,
    /// Target value.
    pub target: f64,
    /// Direction.  `>=` for availability ; `<=` for latency.
    pub direction: SloDirection,
}

/// SLO comparison direction.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SloDirection {
    /// `actual >= target` required (availability).
    AtLeast,
    /// `actual <= target` required (latency).
    AtMost,
}

/// Convergence-based termination.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConvergenceCriteria {
    pub tolerance: f64,
    pub consecutive_iterations: u32,
}

/// Output / record configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutputConfig {
    pub format: OutputFormat,
    pub file: Option<PathBuf>,
    pub record: bool,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self { format: OutputFormat::Ndjson, file: None, record: false }
    }
}

/// OTel + Prometheus observability config.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    pub prometheus: PrometheusConfig,
    pub otel: OtelConfig,
}

/// Prometheus self-metrics endpoint config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrometheusConfig {
    pub listen: String,
    pub enabled: bool,
}

impl Default for PrometheusConfig {
    fn default() -> Self {
        Self { listen: "0.0.0.0:9095".into(), enabled: true }
    }
}

/// OpenTelemetry SDK config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OtelConfig {
    pub endpoint: Option<String>,
    pub sampler_arg: f64,
    pub service_name: String,
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self {
            endpoint: None,
            sampler_arg: 1.0,
            service_name: "efe-meta-swarm-loadgen".into(),
        }
    }
}

/// The merged, validated run-time config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoadgenConfig {
    pub targets: Vec<TargetSpec>,
    pub target_role: TargetRole,
    pub conns_per_target: u32,
    pub per_request_deadline: Option<Duration>,

    pub pattern: PatternSpec,
    pub workload_mix: WorkloadMix,
    pub rate: RateSpec,
    pub termination: TerminationSpec,

    pub tls: Option<TlsConfig>,
    pub observability: ObservabilityConfig,
    pub output: OutputConfig,

    pub seed: u64,
    pub mode: RunMode,
    pub allow_mutation: bool,
    pub chaos_marker: Option<String>,
    pub wait_for_signal: Option<PathBuf>,

    /// Run-level UUID — propagated as `efe.loadgen.run_id` resource attr
    /// and emitted in every result row.
    pub run_id: String,
}

impl LoadgenConfig {
    /// Builder entry-point — used by embedded callers (tests, future SDK).
    #[must_use]
    pub fn builder() -> LoadgenConfigBuilder {
        LoadgenConfigBuilder::default()
    }

    /// Merge CLI args + optional profile into a validated config.
    pub fn from_cli_and_profile(args: RunArgs, profile: Option<Profile>) -> LoadgenResult<Self> {
        args.validate()?;

        // ── Targets ────────────────────────────────────────────────
        let targets = if let Some(p) = &profile {
            // CLI --target wins if non-empty.
            if args.target.is_empty() {
                p.target.endpoints.iter().map(|e| TargetSpec { url: e.url.clone(), weight: e.weight }).collect()
            } else {
                parse_targets(&args.target)?
            }
        } else {
            parse_targets(&args.target)?
        };
        if targets.is_empty() {
            return Err(LoadgenError::InvalidConfig("no targets resolved".into()));
        }

        // ── Pattern ────────────────────────────────────────────────
        let pattern = build_pattern(&args, profile.as_ref())?;

        // ── Workload mix ──────────────────────────────────────────
        let workload_mix = build_workload_mix(&args, profile.as_ref())?;

        // ── Rate ──────────────────────────────────────────────────
        let rate = RateSpec {
            mode: args.rate_mode,
            max_concurrency: args
                .concurrency
                .or_else(|| profile.as_ref().and_then(|p| p.rate.as_ref().and_then(|r| r.max_concurrency)))
                .unwrap_or(200),
            ramp_up: args.ramp_up,
            overflow_policy: OverflowPolicy::Drop,
        };

        // ── Termination ────────────────────────────────────────────
        let termination = TerminationSpec {
            duration: args.duration,
            requests: args.requests,
            until_slo: parse_until_slo(&args.until_slo)?,
            convergence: None,
            shutdown_deadline: args.shutdown_deadline,
        };

        // ── TLS ────────────────────────────────────────────────────
        let tls = build_tls(&args, profile.as_ref())?;

        // ── Observability ─────────────────────────────────────────
        let observability = ObservabilityConfig {
            prometheus: PrometheusConfig {
                listen: args.prom_listen.clone(),
                enabled: true,
            },
            otel: OtelConfig {
                endpoint: args.otel_endpoint.clone(),
                sampler_arg: args.otel_sampler,
                service_name: args.otel_service_name.clone(),
            },
        };

        // ── Output ─────────────────────────────────────────────────
        let output = OutputConfig {
            format: args.output_format,
            file: args.output_file.clone(),
            record: args.record,
        };

        // ── Seed ───────────────────────────────────────────────────
        let seed = args.seed.unwrap_or_else(default_seed);

        Ok(Self {
            targets,
            target_role: args.target_role,
            conns_per_target: args.conns_per_target,
            per_request_deadline: args.per_request_deadline,
            pattern,
            workload_mix,
            rate,
            termination,
            tls,
            observability,
            output,
            seed,
            mode: args.mode,
            allow_mutation: args.allow_mutation,
            chaos_marker: args.chaos_marker.clone(),
            wait_for_signal: args.wait_for_signal.clone(),
            run_id: uuid::Uuid::new_v4().to_string(),
        })
    }
}

/// Default seed = nanos since UNIX epoch (wall clock).
fn default_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Parse a `--target` argument list into [`TargetSpec`]s.
/// Supports the optional `@<weight>` suffix.
fn parse_targets(raw: &[String]) -> LoadgenResult<Vec<TargetSpec>> {
    raw.iter()
        .map(|t| {
            if let Some((url, w)) = t.rsplit_once('@') {
                let weight = w.parse::<f64>().map_err(|_| {
                    LoadgenError::InvalidConfig(format!("invalid weight in --target={t}"))
                })?;
                if weight <= 0.0 || weight > 1.0 {
                    return Err(LoadgenError::InvalidConfig(format!(
                        "--target weight must be in (0, 1] ; got {weight}"
                    )));
                }
                Ok(TargetSpec { url: url.to_string(), weight })
            } else {
                Ok(TargetSpec { url: t.clone(), weight: 1.0 })
            }
        })
        .collect()
}

/// Parse `--until-slo=<id>=<target>` arguments into [`SloThreshold`]s.
fn parse_until_slo(raw: &[String]) -> LoadgenResult<Vec<SloThreshold>> {
    raw.iter()
        .map(|s| {
            let (id, val) = s.split_once('=').ok_or_else(|| {
                LoadgenError::InvalidConfig(format!("--until-slo malformed : {s}"))
            })?;
            let target: f64 = val.parse().map_err(|_| {
                LoadgenError::InvalidConfig(format!("--until-slo target not a number : {s}"))
            })?;
            // Direction inferred from SLO id : availability = AtLeast,
            // latency / drop-rate = AtMost.
            let direction = if id.contains("availability") {
                SloDirection::AtLeast
            } else {
                SloDirection::AtMost
            };
            Ok(SloThreshold { slo_id: id.to_string(), target, direction })
        })
        .collect()
}

/// Build the pattern spec from CLI args + optional profile.
fn build_pattern(args: &RunArgs, profile: Option<&Profile>) -> LoadgenResult<PatternSpec> {
    // Profile pattern wins if --pattern is at default (constant) and
    // profile sets a non-default pattern.  Explicit CLI --pattern
    // overrides profile.
    if let Some(p) = profile {
        if let Some(pat) = p.pattern.clone() {
            return Ok(pat);
        }
    }
    Ok(match args.pattern {
        PatternKind::Constant => PatternSpec::Constant { qps: args.qps },
        PatternKind::Ramp => PatternSpec::Ramp {
            initial_qps: 1.0,
            final_qps: args.qps,
            shape: RampShape::Linear,
            duration: args.duration.unwrap_or(Duration::from_secs(300)),
        },
        PatternKind::Burst => PatternSpec::Burst {
            baseline_qps: args.qps / 10.0,
            burst_qps: args.qps,
            burst_duration: Duration::from_millis(500),
            burst_interval: Duration::from_secs(60),
        },
        PatternKind::Diurnal => PatternSpec::Diurnal {
            daily_min_qps: args.qps / 10.0,
            daily_max_qps: args.qps,
            period: Duration::from_secs(24 * 3600),
            phase: Duration::ZERO,
        },
        PatternKind::Replay => PatternSpec::Replay {
            file: profile
                .and_then(|p| match &p.pattern {
                    Some(PatternSpec::Replay { file, .. }) => Some(file.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    LoadgenError::InvalidConfig(
                        "--pattern=replay requires a profile with a replay file".into(),
                    )
                })?,
            mode: ReplayPlaybackMode::PreserveSpacing,
            speed: 1.0,
        },
    })
}

/// Build the workload mix from CLI + optional profile.
fn build_workload_mix(args: &RunArgs, profile: Option<&Profile>) -> LoadgenResult<WorkloadMix> {
    // 1. CLI --workload-mix overrides everything.
    if let Some(mix_str) = &args.workload_mix {
        let mut mix = WorkloadMix::default();
        for pair in mix_str.split(',') {
            let (k, v) = pair.split_once('=').ok_or_else(|| {
                LoadgenError::InvalidConfig(format!("--workload-mix malformed pair : {pair}"))
            })?;
            let w: f64 = v.parse().map_err(|_| {
                LoadgenError::InvalidConfig(format!("--workload-mix weight not a number : {v}"))
            })?;
            match k.trim() {
                "iterate" => mix.iterate = w,
                "step" => mix.step = w,
                "reset" => mix.reset = w,
                "state" => mix.state = w,
                other => {
                    return Err(LoadgenError::InvalidConfig(format!(
                        "--workload-mix unknown verb : {other}"
                    )))
                }
            }
        }
        if !mix.is_normalised() {
            return Err(LoadgenError::InvalidConfig(
                "--workload-mix weights must sum to 1.0".into(),
            ));
        }
        return Ok(mix);
    }

    // 2. Profile workload-mix.
    if let Some(p) = profile {
        if let Some(mix) = p.workload_mix() {
            if !mix.is_normalised() {
                return Err(LoadgenError::InvalidConfig(
                    "profile workload mix weights must sum to 1.0".into(),
                ));
            }
            return Ok(mix);
        }
    }

    // 3. Default = single-verb based on --workload.
    Ok(WorkloadMix::single(args.workload))
}

/// Build the TLS config from CLI flags or profile.
fn build_tls(args: &RunArgs, profile: Option<&Profile>) -> LoadgenResult<Option<TlsConfig>> {
    if let (Some(cert), Some(key), Some(ca)) =
        (args.tls_cert.clone(), args.tls_key.clone(), args.tls_ca.clone())
    {
        return Ok(Some(TlsConfig {
            cert_path: cert,
            key_path: key,
            ca_path: ca,
            expected_san: args.expected_san.clone(),
            insecure: args.tls_insecure,
        }));
    }
    if let Some(p) = profile {
        if let Some(tls) = p.tls() {
            return Ok(Some(tls));
        }
    }
    if args.tls_insecure {
        // No file paths but explicit --tls-insecure → degenerate TLS.
        return Ok(Some(TlsConfig {
            insecure: true,
            ..Default::default()
        }));
    }
    Ok(None)
}

// ────────────────────────────────────────────────────────────────
//                          Builder
// ────────────────────────────────────────────────────────────────

/// Builder for [`LoadgenConfig`] used by embedded callers (tests +
/// future SDK).
#[derive(Default, Debug)]
pub struct LoadgenConfigBuilder {
    target: Option<String>,
    qps: Option<f64>,
    duration_secs: Option<u64>,
    seed: Option<u64>,
    mode: Option<RunMode>,
}

impl LoadgenConfigBuilder {
    pub fn target(mut self, url: impl Into<String>) -> Self {
        self.target = Some(url.into());
        self
    }
    pub fn qps(mut self, qps: f64) -> Self {
        self.qps = Some(qps);
        self
    }
    pub fn duration_secs(mut self, s: u64) -> Self {
        self.duration_secs = Some(s);
        self
    }
    pub fn seed(mut self, s: u64) -> Self {
        self.seed = Some(s);
        self
    }
    pub fn mode(mut self, m: RunMode) -> Self {
        self.mode = Some(m);
        self
    }
    pub fn build(self) -> LoadgenResult<LoadgenConfig> {
        let url = self
            .target
            .ok_or_else(|| LoadgenError::InvalidConfig("builder needs a target".into()))?;
        let qps = self.qps.unwrap_or(10.0);
        Ok(LoadgenConfig {
            targets: vec![TargetSpec { url, weight: 1.0 }],
            target_role: TargetRole::Controller,
            conns_per_target: 4,
            per_request_deadline: None,
            pattern: PatternSpec::Constant { qps },
            workload_mix: WorkloadMix::single(WorkloadKind::Iterate),
            rate: RateSpec::default(),
            termination: TerminationSpec {
                duration: self.duration_secs.map(Duration::from_secs),
                shutdown_deadline: Duration::from_secs(5),
                ..Default::default()
            },
            tls: None,
            observability: ObservabilityConfig::default(),
            output: OutputConfig::default(),
            seed: self.seed.unwrap_or_else(default_seed),
            mode: self.mode.unwrap_or(RunMode::Dev),
            allow_mutation: false,
            chaos_marker: None,
            wait_for_signal: None,
            run_id: uuid::Uuid::new_v4().to_string(),
        })
    }
}

// ────────────────────────────────────────────────────────────────
//                              Tests
// ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workload_mix_default_is_normalised_single() {
        let m = WorkloadMix::single(WorkloadKind::Iterate);
        assert!(m.is_normalised());
        assert_eq!(m.iterate, 1.0);
        assert_eq!(m.step + m.reset + m.state, 0.0);
    }

    #[test]
    fn parse_targets_supports_weight_suffix() {
        let raw = vec!["grpc://a:50051@0.5".to_string(), "grpc://b:50051".to_string()];
        let parsed = parse_targets(&raw).unwrap();
        assert_eq!(parsed.len(), 2);
        assert!((parsed[0].weight - 0.5).abs() < 1e-9);
        assert_eq!(parsed[0].url, "grpc://a:50051");
        assert!((parsed[1].weight - 1.0).abs() < 1e-9);
    }

    #[test]
    fn parse_targets_rejects_zero_weight() {
        let raw = vec!["grpc://a:50051@0.0".to_string()];
        assert!(parse_targets(&raw).is_err());
    }

    #[test]
    fn parse_targets_rejects_weight_over_one() {
        let raw = vec!["grpc://a:50051@1.5".to_string()];
        assert!(parse_targets(&raw).is_err());
    }

    #[test]
    fn until_slo_availability_infers_at_least() {
        let raw = vec!["iterate-availability=0.999".to_string()];
        let parsed = parse_until_slo(&raw).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].direction, SloDirection::AtLeast);
        assert!((parsed[0].target - 0.999).abs() < 1e-9);
    }

    #[test]
    fn until_slo_latency_infers_at_most() {
        let raw = vec!["iterate-latency-p95=1.0".to_string()];
        let parsed = parse_until_slo(&raw).unwrap();
        assert_eq!(parsed[0].direction, SloDirection::AtMost);
    }

    #[test]
    fn builder_minimal_config() {
        let cfg = LoadgenConfig::builder()
            .target("grpc://localhost:50051")
            .qps(25.0)
            .duration_secs(60)
            .seed(42)
            .build()
            .unwrap();
        assert_eq!(cfg.targets.len(), 1);
        assert!(matches!(cfg.pattern, PatternSpec::Constant { qps } if (qps - 25.0).abs() < 1e-9));
        assert_eq!(cfg.seed, 42);
        assert_eq!(cfg.termination.duration, Some(Duration::from_secs(60)));
    }
}
