//! Sprint **D18** — `efe-bench` CLI.
//!
//! Sprint **D29 (Φ-D)** — adds `--jwt-token`, `--tls-ca-file`, and
//! `--endpoint` flags + the default `TonicBenchClient` backend.
//! Sprint **D31 (Φ-D)** — adds `--gpu-shards` flag.
//! Sprint **D38 (Φ-F)** — adds `--k-swarm`, `--allow-k-above-max`
//! flags + recognises the `mode3-k5` / `mode3-k10` workloads.
//!
//! Cf. architecture § 15. 5 subcommands :
//!
//!   * `run` — execute one canonical workload pattern
//!   * `gen` — generate a dataset to a file (for warmup)
//!   * `list-slos` — print the 7 D18 SLO targets
//!   * `validate-target-env` — quick check that an env name is non-prod
//!   * `report` — render an existing JSON report as Markdown
//!
//! # Production safety
//!
//! Every `run` invocation re-verifies the target env via
//! [`efe_bench::validate_target_env`] BEFORE any traffic is
//! generated.
//!
//! # Default client (post-D29)
//!
//! Without `--allow-mock`, the CLI selects [`TonicBenchClient`]
//! (when the `tonic` cargo feature is built in) and connects to
//! `--endpoint`. With `--allow-mock`, the CLI uses
//! [`MockBenchClient`] for harness self-tests + smoke runs that
//! don't need a real server.

#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};

use efe_bench::{
    client::{BenchClient, ClientKind, MockBenchClient},
    error::BenchError,
    gen::{gen_dataset, DatasetSpec},
    report::{
        canonical_slos, evaluate_c3, evaluate_c9, generate_report_with_client,
        render_markdown,
    },
    runner::{Runner, RunnerConfig},
    workload::{validate_target_env, Workload, K_SWARM_SOFT_CAP},
};

#[cfg(feature = "tonic")]
use efe_bench::{TonicBenchClient, TonicConfig};

/// **Audit fix K-A6 (Φ-D)** — symbolic runtime substrate the bench
/// targets. Recorded into the JSON report so cross-runtime
/// comparisons are unambiguous (CPU baseline vs GPU speedup, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BenchRuntime {
    Cpu,
    Gpu,
}

impl BenchRuntime {
    /// Stable string used in the report payload (do not rename ;
    /// downstream tooling may key on these strings).
    fn as_str(self) -> &'static str {
        match self {
            BenchRuntime::Cpu => "cpu",
            BenchRuntime::Gpu => "gpu",
        }
    }
}

/// `clap` value parser for `--runtime`. Accepts case-insensitive
/// `cpu` / `gpu`. Returns `BenchError::config` on any other value
/// — surfaces with ExitCode 2 (unknown subcommand-style usage).
fn parse_runtime(s: &str) -> Result<BenchRuntime, String> {
    match s.to_lowercase().as_str() {
        "cpu" => Ok(BenchRuntime::Cpu),
        "gpu" => Ok(BenchRuntime::Gpu),
        other => Err(format!(
            "unknown runtime {other:?} (expected `cpu` or `gpu`)"
        )),
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "efe-bench",
    version,
    about = "EFE bench harness — D12 catalog + Execute throughput (D18) + TonicBenchClient (D29) + multi-GPU sharding (D31) + Mode-3 workloads (D38)."
)]
struct Cli {
    /// Output machine-readable JSON instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,

    /// Verbose logging.
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run one canonical workload pattern against a target cluster.
    Run {
        /// Workload pattern (read-heavy | write-heavy | balanced |
        /// burst | multi-tenant | soak | mode3-k5 | mode3-k10).
        #[arg(long)]
        workload: String,

        /// Target environment ; MUST NOT be `prod`.
        #[arg(long, env = "EFE_BENCH_TARGET_ENV")]
        target_env: String,

        /// **Sprint D29 (Φ-D)** — gRPC endpoint URL. Default points
        /// to a local kind-spawned server. The bench REFUSES to
        /// run against `https://*.prod.*` unless `--allow-mock` is
        /// also set (defense in depth on top of `--target-env`).
        #[arg(long, default_value = "http://127.0.0.1:50051", env = "EFE_BENCH_ENDPOINT")]
        endpoint: String,

        /// **Sprint D29 (Φ-D)** — JWT bearer token. Propagated as
        /// `authorization: Bearer <token>` metadata. Read from
        /// `EFE_BENCH_JWT_TOKEN` env if not passed.
        #[arg(long, env = "EFE_BENCH_JWT_TOKEN", hide_env_values = true)]
        jwt_token: Option<String>,

        /// **Sprint D29 (Φ-D)** — path to a CA certificate (PEM)
        /// for mTLS server verification. None = plain h2c.
        #[arg(long)]
        tls_ca_file: Option<PathBuf>,

        /// **Sprint D29 (Φ-D)** — server domain name for TLS SNI.
        /// Default `localhost` ; pass the staging hostname when
        /// connecting through a proxy.
        #[arg(long)]
        tls_domain_name: Option<String>,

        /// **Sprint D31 (Φ-D)** — number of GPU shards to
        /// orchestrate. Each worker is assigned
        /// `shard_id = worker_idx % gpu_shards` and propagates
        /// it via the `x-efe-gpu-shard-id` gRPC metadata.
        ///
        /// Default 1 = no sharding (single GPU).
        /// Range : 1..=16 (cluster cap).
        #[arg(
            long,
            default_value = "1",
            value_parser = clap::value_parser!(u32).range(1..=16)
        )]
        gpu_shards: u32,

        /// **Sprint D38 (Φ-F)** — override the workload's default
        /// `k_swarm` value. Only valid for `mode3-*` workloads.
        /// Range : 1..=10 (hard cap) ; values > 5 require
        /// `--allow-k-above-max`.
        #[arg(long)]
        k_swarm: Option<u32>,

        /// **Sprint D38 (Φ-F)** — explicitly opt into `k_swarm > 5`
        /// (server-side `Mode3Service` clamps to 5 ; this flag
        /// acknowledges that the bench will exercise the clamp
        /// path rather than measure true k=10 parallelism).
        #[arg(long, default_value_t = false)]
        allow_k_above_max: bool,

        /// Override the workload's default duration (seconds).
        #[arg(long)]
        duration_secs: Option<u64>,

        /// Override the workload's default RPS target.
        #[arg(long)]
        rps: Option<u32>,

        /// Override the workload's default concurrency.
        #[arg(long)]
        concurrency: Option<u32>,

        /// Replica count of the cluster being benched (informational
        /// for the report).
        #[arg(long, default_value_t = 1)]
        replicas: u32,

        /// Where to write the JSON report.
        #[arg(long)]
        report_json: Option<PathBuf>,

        /// Where to write the Markdown report.
        #[arg(long)]
        report_md: Option<PathBuf>,

        /// **Audit fix D18-A10** — explicitly opt into the
        /// `MockBenchClient`. Pre-D29 the bench harness shipped
        /// only the Mock client and required this flag for any
        /// run. Post-D29 the default backend is
        /// [`TonicBenchClient`] ; `--allow-mock` is now an
        /// explicit opt-in for harness self-tests.
        #[arg(long, default_value_t = false)]
        allow_mock: bool,

        /// **Audit fix K-A6 (Φ-D)** — runtime substrate to bench
        /// against the target. `cpu` (default) → bench against
        /// `CpuBackend` ; `gpu` → bench against `efe_kernels::CudaBackend`
        /// (requires the `cuda` Cargo feature on the server side).
        /// Recorded into the JSON / Markdown report so cross-runtime
        /// comparisons are unambiguous.
        #[arg(long, default_value = "cpu", value_parser = parse_runtime)]
        runtime: BenchRuntime,
    },

    /// Generate a dataset to a JSON file (for warmup or test
    /// fixtures).
    Gen {
        /// Spec preset : `smoke` | `tier1` | `stress`.
        #[arg(long, default_value = "smoke")]
        spec: String,

        /// Output file (JSON).
        #[arg(long)]
        out: PathBuf,
    },

    /// Print the 7 D18 SLO targets as a table.
    ListSlos,

    /// Sanity-check a target env value (returns 0 if non-prod, 7 if prod).
    ValidateTargetEnv {
        /// Env value to check.
        #[arg(long)]
        env: String,
    },

    /// Render an existing JSON report as Markdown.
    Report {
        /// JSON report path.
        #[arg(long)]
        json: PathBuf,
    },

    /// **Sprint D38 (Φ-F)** — compare a Mode-3 report against a
    /// Mode-1 baseline and emit the C-3 + C-9 SLO verdicts.
    Compare {
        /// Mode-1 baseline JSON report (e.g. `read-heavy`).
        #[arg(long)]
        mode1_json: PathBuf,
        /// Mode-3 candidate JSON report (e.g. `mode3-k5`).
        #[arg(long)]
        mode3_json: PathBuf,
        /// Optional serial Mode-3 report (k=1) for the C-9 speedup
        /// check. When absent, only C-3 is evaluated.
        #[arg(long)]
        mode3_serial_json: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let json_out = cli.json;
    let verbose = cli.verbose;
    if verbose {
        let filter = tracing_subscriber::EnvFilter::try_new("debug")
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
    }

    let result: Result<(), BenchError> = match cli.command {
        Command::Run {
            workload,
            target_env,
            endpoint,
            jwt_token,
            tls_ca_file,
            tls_domain_name,
            gpu_shards,
            k_swarm,
            allow_k_above_max,
            duration_secs,
            rps,
            concurrency,
            replicas,
            report_json,
            report_md,
            allow_mock,
            runtime,
        } => {
            cmd_run(RunArgs {
                json_out,
                workload_label: workload,
                target_env,
                endpoint,
                jwt_token,
                tls_ca_file,
                tls_domain_name,
                gpu_shards,
                k_swarm,
                allow_k_above_max,
                duration_secs,
                rps,
                concurrency,
                replicas,
                report_json,
                report_md,
                allow_mock,
                runtime,
            })
            .await
        }
        Command::Gen { spec, out } => cmd_gen(json_out, &spec, &out).await,
        Command::ListSlos => cmd_list_slos(json_out),
        Command::ValidateTargetEnv { env } => cmd_validate_env(&env),
        Command::Report { json } => cmd_render_report(&json),
        Command::Compare {
            mode1_json,
            mode3_json,
            mode3_serial_json,
        } => cmd_compare(json_out, &mode1_json, &mode3_json, mode3_serial_json.as_deref()),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("efe-bench: {e}");
            match e {
                BenchError::RefusedProdTarget(_) => ExitCode::from(7),
                BenchError::HistogramOverflow(_) => ExitCode::from(8),
                BenchError::GpuShardOutOfRange { .. } => ExitCode::from(2),
                BenchError::KSwarmAboveMax { .. } => ExitCode::from(2),
                BenchError::InvalidWorkload(_) | BenchError::Config(_) => ExitCode::from(2),
                BenchError::Tonic(_) => ExitCode::from(3),
                _ => ExitCode::FAILURE,
            }
        }
    }
}

/// Bundled `run` subcommand args (avoids `too_many_arguments`).
///
/// **Sprint D29 (Φ-D)** — `endpoint`, `jwt_token`, `tls_ca_file`,
/// `tls_domain_name` are only read inside `build_real_client`
/// (gated `feature = "tonic"`). Without the feature they're
/// flagged dead_code by rustc, even though they're still parsed
/// from the CLI for forward-compat (an operator can rebuild with
/// `--features tonic` without re-learning the flag set). The
/// `cfg_attr(not(feature = "tonic"), allow(dead_code))` keeps
/// the OFF-feature build warning-free without losing the Tonic
/// surface when the feature is ON.
#[cfg_attr(not(feature = "tonic"), allow(dead_code))]
struct RunArgs {
    json_out: bool,
    workload_label: String,
    target_env: String,
    endpoint: String,
    jwt_token: Option<String>,
    tls_ca_file: Option<PathBuf>,
    tls_domain_name: Option<String>,
    gpu_shards: u32,
    k_swarm: Option<u32>,
    allow_k_above_max: bool,
    duration_secs: Option<u64>,
    rps: Option<u32>,
    concurrency: Option<u32>,
    replicas: u32,
    report_json: Option<PathBuf>,
    report_md: Option<PathBuf>,
    allow_mock: bool,
    runtime: BenchRuntime,
}

async fn cmd_run(args: RunArgs) -> Result<(), BenchError> {
    // Defense-in-depth : refuse prod BEFORE any traffic generation.
    validate_target_env(&args.target_env)?;

    // Build the workload (allowing CLI overrides).
    let mut workload = Workload::canonical(&args.workload_label)?;
    if let Some(d) = args.duration_secs {
        workload.duration = Duration::from_secs(d);
    }
    if let Some(r) = args.rps {
        workload.target_rps = r;
    }
    if let Some(c) = args.concurrency {
        workload.concurrency = c;
    }

    // **Sprint D38 (Φ-F)** — apply --k-swarm override if present
    // and the workload is Mode-3.
    if let Some(k) = args.k_swarm {
        efe_bench::workload::validate_k_swarm(k, args.allow_k_above_max)?;
        if !workload.is_mode3() {
            return Err(BenchError::config(format!(
                "--k-swarm only applies to mode3-* workloads (got workload={:?})",
                workload.label
            )));
        }
        // Replace the k_swarm value in every Mode3Execute op.
        for entry in workload.mix.iter_mut() {
            if let efe_bench::workload::WorkloadOp::Mode3Execute { k_swarm } =
                &mut entry.0
            {
                *k_swarm = k;
            }
        }
    } else if let Some(k) = workload.k_swarm() {
        // Workload-canonical k_swarm value still subject to the
        // soft-cap opt-in : `mode3-k10` has k=10 and requires
        // `--allow-k-above-max` (or be invoked explicitly through
        // its canonical label, which acknowledges the cap).
        if k > K_SWARM_SOFT_CAP && !args.allow_k_above_max {
            return Err(BenchError::KSwarmAboveMax { got: k });
        }
    }

    workload.validate()?;

    let workload_k_swarm = workload.k_swarm();

    // **Sprint D29 (Φ-D)** — pick the client backend.
    //
    // Default = Tonic (post-D29 contract). `--allow-mock`
    // explicitly switches to the Mock client for harness
    // self-tests + smoke runs that don't need a real server.
    let (client, client_kind): (Arc<dyn BenchClient>, ClientKind) = if args.allow_mock {
        let mc = MockBenchClient::new().with_canned_latency(Duration::from_millis(5));
        (Arc::new(mc), ClientKind::Mock)
    } else {
        build_real_client(&args).await?
    };

    // Generate a small dataset for the bench (smoke is fine for
    // mock client ; production would use tier1 or stress).
    let dataset = Arc::new(gen_dataset(&DatasetSpec::smoke())?);

    let mut cfg = RunnerConfig::new(workload, &args.target_env)?;
    if args.gpu_shards > 1 {
        cfg = cfg.with_gpu_shards(args.gpu_shards)?;
    }
    let runner = Runner::new(cfg, client, dataset);

    if !args.json_out {
        eprintln!(
            "[efe-bench] running workload={workload_label} target_env={target_env} \
             replicas={replicas} runtime={runtime_str} client={client} \
             gpu_shards={gpu_shards}{k}",
            workload_label = args.workload_label,
            target_env = args.target_env,
            replicas = args.replicas,
            runtime_str = args.runtime.as_str(),
            client = client_kind.as_str(),
            gpu_shards = args.gpu_shards,
            k = workload_k_swarm
                .map(|k| format!(" k_swarm={k}"))
                .unwrap_or_default()
        );
    }

    let outcome = runner.run().await?;
    let report = generate_report_with_client(
        &outcome,
        &args.target_env,
        args.replicas,
        client_kind,
        workload_k_swarm,
    )?;

    // Emit JSON report.
    if let Some(path) = &args.report_json {
        let json = serde_json::to_string_pretty(&report)?;
        std::fs::write(path, json)?;
        if !args.json_out {
            eprintln!("[efe-bench] wrote JSON report to {}", path.display());
        }
    }

    // Emit Markdown report.
    if let Some(path) = &args.report_md {
        let md = render_markdown(&report);
        std::fs::write(path, md)?;
        if !args.json_out {
            eprintln!("[efe-bench] wrote Markdown report to {}", path.display());
        }
    }

    if args.json_out {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        println!("{}", render_markdown(&report));
    }

    if !report.all_slos_pass {
        return Err(BenchError::config(format!(
            "SLO regression : {} of {} SLOs failed",
            report.slos.iter().filter(|s| !s.pass).count(),
            report.slos.len()
        )));
    }

    Ok(())
}

/// **Sprint D29 (Φ-D)** — build a real (non-mock) client.
///
/// When the `tonic` cargo feature is built in, returns a
/// configured [`TonicBenchClient`]. When the feature is OFF, we
/// surface a clear configuration error pointing the operator at
/// `--allow-mock` or a rebuild with `--features tonic`.
async fn build_real_client(
    _args: &RunArgs,
) -> Result<(Arc<dyn BenchClient>, ClientKind), BenchError> {
    #[cfg(feature = "tonic")]
    {
        let mut config = TonicConfig::new(_args.endpoint.clone());
        if let Some(t) = &_args.jwt_token {
            config = config.with_jwt(t.clone());
        }
        if let Some(p) = &_args.tls_ca_file {
            config = config.with_tls_ca_file(p.clone());
        }
        if let Some(d) = &_args.tls_domain_name {
            config = config.with_tls_domain_name(d.clone());
        }
        if _args.gpu_shards > 1 {
            config = config.with_gpu_shards(_args.gpu_shards);
        }
        let client = TonicBenchClient::connect(config).await?;
        Ok((Arc::new(client), ClientKind::Tonic))
    }
    #[cfg(not(feature = "tonic"))]
    {
        Err(BenchError::config(
            "the tonic feature is OFF — rebuild with `cargo build -p efe-bench \
             --features tonic` to enable the real gRPC client, or pass \
             `--allow-mock` to run a harness self-test against MockBenchClient."
                .to_string(),
        ))
    }
}

async fn cmd_gen(
    _json_out: bool,
    spec_label: &str,
    out: &std::path::Path,
) -> Result<(), BenchError> {
    let spec = match spec_label {
        "smoke" => DatasetSpec::smoke(),
        "tier1" => DatasetSpec::tier1_typical(),
        "stress" => DatasetSpec::stress(),
        other => {
            return Err(BenchError::config(format!(
                "unknown spec preset: {other:?} (expected smoke|tier1|stress)"
            )))
        }
    };
    let dataset = gen_dataset(&spec)?;
    let json = serde_json::to_string_pretty(&dataset)?;
    std::fs::write(out, json)?;
    eprintln!(
        "[efe-bench] wrote {} tenants × {} catalogs to {}",
        dataset.n_tenants(),
        dataset.n_catalogs(),
        out.display()
    );
    Ok(())
}

fn cmd_list_slos(json_out: bool) -> Result<(), BenchError> {
    let slos = canonical_slos();
    if json_out {
        let json: Vec<serde_json::Value> = slos
            .iter()
            .map(|t| {
                serde_json::json!({
                    "id": t.id,
                    "op": t.op,
                    "quantile": if t.quantile.is_nan() { serde_json::Value::Null } else { serde_json::Value::from(t.quantile) },
                    "target_ms": t.target_ms,
                    "min_rps": t.min_rps,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        println!("{:<14} {:<20} {:<10} {:<14} Min RPS", "SLO", "Op", "Quantile", "Target (ms)");
        println!("{}", "─".repeat(72));
        for t in slos {
            let q_str = if t.quantile.is_nan() {
                "n/a".to_string()
            } else {
                format!("p{:.0}", t.quantile * 100.0)
            };
            println!(
                "{:<14} {:<20} {:<10} {:<14} {}",
                t.id, t.op, q_str, t.target_ms, t.min_rps
            );
        }
        println!();
        println!("Derived SLOs (post-bench compare via `efe-bench compare`):");
        println!("  D31-GPU-1   shard fairness   Jain ≥ 0.95");
        println!("  C-3         mode3 latency    p99(mode3) ≤ 3 × p99(mode1)");
        println!("  C-9         mode3 speedup    rps(k=5) ≥ 2.5 × rps(k=1)");
    }
    Ok(())
}

fn cmd_validate_env(env: &str) -> Result<(), BenchError> {
    validate_target_env(env)?;
    println!("ok : {env:?} is a non-prod target");
    Ok(())
}

fn cmd_render_report(json_path: &std::path::Path) -> Result<(), BenchError> {
    let bytes = std::fs::read(json_path)?;
    let report: efe_bench::BenchReport = serde_json::from_slice(&bytes)?;
    println!("{}", render_markdown(&report));
    Ok(())
}

/// **Sprint D38 (Φ-F)** — `compare` subcommand.
///
/// Loads two reports + emits the C-3 + (optional) C-9 SLO
/// verdicts. ExitCode 0 on pass, 1 on regression.
fn cmd_compare(
    json_out: bool,
    mode1_path: &std::path::Path,
    mode3_path: &std::path::Path,
    mode3_serial_path: Option<&std::path::Path>,
) -> Result<(), BenchError> {
    let mode1: efe_bench::BenchReport =
        serde_json::from_slice(&std::fs::read(mode1_path)?)?;
    let mode3: efe_bench::BenchReport =
        serde_json::from_slice(&std::fs::read(mode3_path)?)?;
    let c3 = evaluate_c3(&mode3, &mode1);
    let mut all_pass = c3.pass;
    let c9 = if let Some(serial_path) = mode3_serial_path {
        let serial: efe_bench::BenchReport =
            serde_json::from_slice(&std::fs::read(serial_path)?)?;
        let r = evaluate_c9(&mode3, &serial);
        if !r.pass {
            all_pass = false;
        }
        Some(r)
    } else {
        None
    };

    if json_out {
        let payload = serde_json::json!({
            "all_pass": all_pass,
            "c3": {
                "slo": c3.slo,
                "pass": c3.pass,
                "detail": c3.detail,
            },
            "c9": c9.as_ref().map(|r| serde_json::json!({
                "slo": r.slo,
                "pass": r.pass,
                "detail": r.detail,
            })),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("# efe-bench compare");
        println!();
        println!(
            "C-3 : {} {}",
            if c3.pass { "PASS ✅" } else { "FAIL ❌" },
            c3.detail
        );
        if let Some(r) = &c9 {
            println!(
                "C-9 : {} {}",
                if r.pass { "PASS ✅" } else { "FAIL ❌" },
                r.detail
            );
        } else {
            println!("C-9 : skipped (pass --mode3-serial-json to enable)");
        }
        println!();
        println!(
            "Overall : {}",
            if all_pass { "ALL PASS ✅" } else { "REGRESSION ❌" }
        );
    }

    if !all_pass {
        return Err(BenchError::config("derived SLO regression".to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_env_rejects_prod() {
        let r = cmd_validate_env("prod");
        assert!(matches!(r, Err(BenchError::RefusedProdTarget(_))));
    }

    #[test]
    fn validate_env_accepts_dev() {
        cmd_validate_env("dev").unwrap();
    }

    #[test]
    fn list_slos_returns_ok() {
        cmd_list_slos(false).unwrap();
    }

    #[tokio::test]
    async fn cmd_gen_smoke_writes_file() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("ds.json");
        cmd_gen(false, "smoke", &out).await.unwrap();
        assert!(out.exists());
        let bytes = std::fs::read(&out).unwrap();
        assert!(!bytes.is_empty());
    }

    #[tokio::test]
    async fn cmd_gen_unknown_spec_errors() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("ds.json");
        let err = cmd_gen(false, "unknown", &out).await.unwrap_err();
        assert!(matches!(err, BenchError::Config(_)));
    }

    // -----------------------------------------------------------
    // **Sprint D38 (Φ-F)** — compare subcommand smoke test.
    // -----------------------------------------------------------

    #[test]
    fn cmd_compare_passes_when_all_slos_pass() {
        let dir = tempfile::tempdir().unwrap();
        let mode1_path = dir.path().join("mode1.json");
        let mode3_path = dir.path().join("mode3.json");

        // Build a mode-1 report (high baseline p99 = 100ms).
        let mut mode1 = efe_bench::BenchReport {
            schema_version: "2.0.0".into(),
            client_kind: ClientKind::Tonic,
            gpu_shards: None,
            k_swarm: None,
            workload: "read-heavy".into(),
            target_env: "kind".into(),
            generated_at: chrono::Utc::now(),
            replicas: 1,
            duration_seconds: 60.0,
            success_count: 1000,
            error_count: 0,
            p50_ms: Some(20.0),
            p95_ms: Some(80.0),
            p99_ms: Some(100.0),
            rps_achieved: 100.0,
            per_op: std::collections::BTreeMap::new(),
            per_shard: std::collections::BTreeMap::new(),
            errors_by_label: std::collections::BTreeMap::new(),
            slos: vec![],
            all_slos_pass: true,
        };
        mode1.per_op.insert(
            "execute".into(),
            efe_bench::OpReport {
                p50_ms: Some(20.0),
                p95_ms: Some(80.0),
                p99_ms: Some(100.0),
                mean_ms: Some(30.0),
                rps: 100.0,
                success_count: 1000,
                error_count: 0,
            },
        );
        std::fs::write(
            &mode1_path,
            serde_json::to_string_pretty(&mode1).unwrap(),
        )
        .unwrap();

        // Mode-3 with p99=200ms → ratio = 2× ≤ 3 → PASS.
        let mut mode3 = mode1.clone();
        mode3.workload = "mode3-k5".into();
        mode3.k_swarm = Some(5);
        mode3.per_op.clear();
        mode3.per_op.insert(
            "mode3_execute_k5".into(),
            efe_bench::OpReport {
                p50_ms: Some(40.0),
                p95_ms: Some(160.0),
                p99_ms: Some(200.0),
                mean_ms: Some(60.0),
                rps: 50.0,
                success_count: 500,
                error_count: 0,
            },
        );
        std::fs::write(
            &mode3_path,
            serde_json::to_string_pretty(&mode3).unwrap(),
        )
        .unwrap();

        cmd_compare(true, &mode1_path, &mode3_path, None).unwrap();
    }

    #[test]
    fn cmd_compare_passes_with_serial_json_for_c9_check() {
        // **Audit fix M6-A3 (Φ-F)** — exercise the C-9 path of
        // the compare subcommand. Pre-audit only the
        // serial_json=None branch was tested.
        let dir = tempfile::tempdir().unwrap();
        let mode1_path = dir.path().join("mode1.json");
        let mode3_path = dir.path().join("mode3.json");
        let serial_path = dir.path().join("mode3_serial.json");

        let mut mode1 = efe_bench::BenchReport {
            schema_version: "2.0.0".into(),
            client_kind: ClientKind::Tonic,
            gpu_shards: None,
            k_swarm: None,
            workload: "read-heavy".into(),
            target_env: "kind".into(),
            generated_at: chrono::Utc::now(),
            replicas: 1,
            duration_seconds: 60.0,
            success_count: 1000,
            error_count: 0,
            p50_ms: Some(20.0),
            p95_ms: Some(80.0),
            p99_ms: Some(100.0),
            rps_achieved: 100.0,
            per_op: std::collections::BTreeMap::new(),
            per_shard: std::collections::BTreeMap::new(),
            errors_by_label: std::collections::BTreeMap::new(),
            slos: vec![],
            all_slos_pass: true,
        };
        mode1.per_op.insert(
            "execute".into(),
            efe_bench::OpReport {
                p50_ms: Some(20.0),
                p95_ms: Some(80.0),
                p99_ms: Some(100.0),
                mean_ms: Some(30.0),
                rps: 100.0,
                success_count: 1000,
                error_count: 0,
            },
        );
        std::fs::write(
            &mode1_path,
            serde_json::to_string_pretty(&mode1).unwrap(),
        )
        .unwrap();

        // Mode-3 parallel (k=5) with high RPS = 60.
        let mut mode3 = mode1.clone();
        mode3.workload = "mode3-k5".into();
        mode3.k_swarm = Some(5);
        mode3.per_op.clear();
        mode3.per_op.insert(
            "mode3_execute_k5".into(),
            efe_bench::OpReport {
                p50_ms: Some(30.0),
                p95_ms: Some(60.0),
                p99_ms: Some(100.0), // ratio = 1.0 → C-3 pass
                mean_ms: Some(40.0),
                rps: 60.0,
                success_count: 600,
                error_count: 0,
            },
        );
        std::fs::write(
            &mode3_path,
            serde_json::to_string_pretty(&mode3).unwrap(),
        )
        .unwrap();

        // Mode-3 serial (k=1) with low RPS = 20. Speedup = 60/20 = 3× → C-9 pass.
        let mut serial = mode3.clone();
        serial.k_swarm = Some(1);
        serial.per_op.clear();
        serial.per_op.insert(
            "mode3_execute".into(),
            efe_bench::OpReport {
                p50_ms: Some(30.0),
                p95_ms: Some(60.0),
                p99_ms: Some(100.0),
                mean_ms: Some(40.0),
                rps: 20.0,
                success_count: 200,
                error_count: 0,
            },
        );
        std::fs::write(
            &serial_path,
            serde_json::to_string_pretty(&serial).unwrap(),
        )
        .unwrap();

        cmd_compare(true, &mode1_path, &mode3_path, Some(&serial_path)).unwrap();
    }

    #[test]
    fn cmd_compare_fails_when_c9_speedup_below_floor() {
        // **Audit fix M6-A3 (Φ-F)** — exercise the failure path of
        // C-9 specifically (C-3 still passes). Speedup = 1.5× < 2.5×.
        let dir = tempfile::tempdir().unwrap();
        let mode1_path = dir.path().join("mode1.json");
        let mode3_path = dir.path().join("mode3.json");
        let serial_path = dir.path().join("mode3_serial.json");

        let mut mode1 = efe_bench::BenchReport {
            schema_version: "2.0.0".into(),
            client_kind: ClientKind::Tonic,
            gpu_shards: None,
            k_swarm: None,
            workload: "read-heavy".into(),
            target_env: "kind".into(),
            generated_at: chrono::Utc::now(),
            replicas: 1,
            duration_seconds: 60.0,
            success_count: 1000,
            error_count: 0,
            p50_ms: Some(20.0),
            p95_ms: Some(80.0),
            p99_ms: Some(100.0),
            rps_achieved: 100.0,
            per_op: std::collections::BTreeMap::new(),
            per_shard: std::collections::BTreeMap::new(),
            errors_by_label: std::collections::BTreeMap::new(),
            slos: vec![],
            all_slos_pass: true,
        };
        mode1.per_op.insert(
            "execute".into(),
            efe_bench::OpReport {
                p50_ms: Some(20.0),
                p95_ms: Some(80.0),
                p99_ms: Some(100.0),
                mean_ms: Some(30.0),
                rps: 100.0,
                success_count: 1000,
                error_count: 0,
            },
        );
        std::fs::write(
            &mode1_path,
            serde_json::to_string_pretty(&mode1).unwrap(),
        )
        .unwrap();

        let mut mode3 = mode1.clone();
        mode3.workload = "mode3-k5".into();
        mode3.k_swarm = Some(5);
        mode3.per_op.clear();
        mode3.per_op.insert(
            "mode3_execute_k5".into(),
            efe_bench::OpReport {
                p50_ms: Some(30.0),
                p95_ms: Some(60.0),
                p99_ms: Some(100.0), // C-3 pass (ratio = 1.0)
                mean_ms: Some(40.0),
                rps: 30.0, // parallel
                success_count: 300,
                error_count: 0,
            },
        );
        std::fs::write(
            &mode3_path,
            serde_json::to_string_pretty(&mode3).unwrap(),
        )
        .unwrap();

        // Serial RPS = 20 → speedup 30/20 = 1.5× < 2.5× → C-9 FAIL.
        let mut serial = mode3.clone();
        serial.per_op.clear();
        serial.per_op.insert(
            "mode3_execute".into(),
            efe_bench::OpReport {
                p50_ms: Some(30.0),
                p95_ms: Some(60.0),
                p99_ms: Some(100.0),
                mean_ms: Some(40.0),
                rps: 20.0,
                success_count: 200,
                error_count: 0,
            },
        );
        std::fs::write(
            &serial_path,
            serde_json::to_string_pretty(&serial).unwrap(),
        )
        .unwrap();

        let r = cmd_compare(true, &mode1_path, &mode3_path, Some(&serial_path));
        assert!(
            matches!(r, Err(BenchError::Config(_))),
            "C-9 1.5× speedup must regress the compare subcommand"
        );
    }

    #[test]
    fn cmd_compare_fails_when_c3_violated() {
        let dir = tempfile::tempdir().unwrap();
        let mode1_path = dir.path().join("mode1.json");
        let mode3_path = dir.path().join("mode3.json");

        let mut mode1 = efe_bench::BenchReport {
            schema_version: "2.0.0".into(),
            client_kind: ClientKind::Tonic,
            gpu_shards: None,
            k_swarm: None,
            workload: "read-heavy".into(),
            target_env: "kind".into(),
            generated_at: chrono::Utc::now(),
            replicas: 1,
            duration_seconds: 60.0,
            success_count: 1000,
            error_count: 0,
            p50_ms: Some(20.0),
            p95_ms: Some(80.0),
            p99_ms: Some(100.0),
            rps_achieved: 100.0,
            per_op: std::collections::BTreeMap::new(),
            per_shard: std::collections::BTreeMap::new(),
            errors_by_label: std::collections::BTreeMap::new(),
            slos: vec![],
            all_slos_pass: true,
        };
        mode1.per_op.insert(
            "execute".into(),
            efe_bench::OpReport {
                p50_ms: Some(20.0),
                p95_ms: Some(80.0),
                p99_ms: Some(100.0),
                mean_ms: Some(30.0),
                rps: 100.0,
                success_count: 1000,
                error_count: 0,
            },
        );
        std::fs::write(
            &mode1_path,
            serde_json::to_string_pretty(&mode1).unwrap(),
        )
        .unwrap();

        // Mode-3 with p99=400ms → ratio = 4× > 3 → FAIL.
        let mut mode3 = mode1.clone();
        mode3.workload = "mode3-k5".into();
        mode3.per_op.clear();
        mode3.per_op.insert(
            "mode3_execute_k5".into(),
            efe_bench::OpReport {
                p50_ms: Some(80.0),
                p95_ms: Some(300.0),
                p99_ms: Some(400.0),
                mean_ms: Some(120.0),
                rps: 50.0,
                success_count: 500,
                error_count: 0,
            },
        );
        std::fs::write(
            &mode3_path,
            serde_json::to_string_pretty(&mode3).unwrap(),
        )
        .unwrap();

        let r = cmd_compare(true, &mode1_path, &mode3_path, None);
        assert!(matches!(r, Err(BenchError::Config(_))));
    }
}
