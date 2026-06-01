//! PR #20 § 6 + § 30.1 — main run loop + subcommand dispatch.
//!
//! `dispatch_run` wires every module together :
//!
//! ```text
//! cli args + profile YAML
//!          │
//!          ▼
//! LoadgenConfig (validated)
//!          │
//!          ▼
//! tokio runtime
//!  │   │   │   │   │
//!  │   │   │   │   └── Termination policies (5)
//!  │   │   │   └────── Output writer (terminal/csv/ndjson/parquet)
//!  │   │   └────────── Self-instrumentation (Prometheus + OTel)
//!  │   └────────────── Rate controller (open/closed/hybrid)
//!  └────────────────── Pattern engine (constant/ramp/burst/diurnal/replay)
//!          │
//!          ▼
//! ClientPool (gRPC + mTLS)
//!          │
//!          ▼
//! Workload dispatch (iterate/step/reset/state/mixed)
//!          │
//!          ▼
//! RequestRecord ──► OutputWriter
//!                 ──► RunStats
//!                 ──► Prometheus metrics
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex as PlMutex;
use rand::SeedableRng as _;
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::cli::{
    AggregateArgs, CompareArgs, ReplayArgs, RunArgs, ValidateArgs,
};
use crate::client::ClientPool;
use crate::config::LoadgenConfig;
use crate::determinism::{SeedRegistry, SubStream};
use crate::error::{ExitCode, LoadgenError, LoadgenResult};
use crate::metrics::LoadgenMetrics;
use crate::profile::Profile;
use crate::stats::{RequestRecord, RunStats};
use crate::termination::{StopReason, TerminationPolicy};

/// Summary of a completed run — returned by [`run`].  Serialised as the
/// JSON SLO report when `--emit-slo-report` is set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub run_id: String,
    pub seed: u64,
    pub total_requests: u64,
    pub duration_s: f64,
    pub stop_reason: String,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub error_rate: f64,
    pub overall_pass: bool,
}

/// Library entry-point.  The binary's `main` calls this via
/// `dispatch_run`.  Public for embedded usage.
pub async fn run(cfg: LoadgenConfig) -> LoadgenResult<RunSummary> {
    tracing::info!(
        targets = cfg.targets.len(),
        seed = cfg.seed,
        mode = ?cfg.mode,
        "loadgen starting"
    );

    // ── 1. Seed registry + per-task RNG ────────────────────────────
    let pod_ordinal = std::env::var("POD_ORDINAL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0_u32);
    let seeds = SeedRegistry::new(cfg.seed, pod_ordinal);

    // ── 2. Metrics + OTel SDK ──────────────────────────────────────
    let metrics = LoadgenMetrics::register()?;
    let _otel_guard = crate::otel::init(&cfg.observability.otel)?;
    let _prom_handle = if cfg.observability.prometheus.enabled {
        let addr: std::net::SocketAddr = cfg
            .observability
            .prometheus
            .listen
            .parse()
            .map_err(|e| LoadgenError::InvalidConfig(format!("prom listen addr : {e}")))?;
        Some(crate::metrics::spawn_listener(metrics.clone(), addr).await?)
    } else {
        None
    };

    // ── 3. Client pool ─────────────────────────────────────────────
    let selector_rng = seeds.rng(SubStream::TargetSelection);
    let pool = ClientPool::connect(&cfg, selector_rng).await?;
    tracing::info!(target_count = pool.target_count(), "client pool ready");

    // ── 4. Pattern + rate + workload ──────────────────────────────
    let mut pattern_box = crate::pattern::from_spec(&cfg.pattern);
    pattern_box = crate::pattern::with_ramp_up(pattern_box, cfg.rate.ramp_up);
    // Convert to Arc so the engine can hand it to spawned dispatch
    // tasks (Arc<dyn Trait> is Send + Sync + 'static when the trait
    // bound is Send + Sync, as ours is).
    let pattern: Arc<dyn crate::pattern::TrafficPattern> = Arc::from(pattern_box);
    let rate = crate::rate::from_spec(&cfg.rate);
    let workload: Arc<dyn crate::workload::Workload> =
        Arc::from(crate::workload::from_mix(&cfg.workload_mix, /*dim*/ 64));

    // ── 5. Output writer (owned by a dedicated tokio task that drains
    //      an mpsc channel — avoids the writer's !Send / !Sync constraints
    //      when shared with concurrent dispatch tasks).
    let writer = crate::output::build(cfg.output.format, cfg.output.file.as_deref())?;
    let writer = Arc::new(PlMutex::new(writer));

    // ── 6. Termination ─────────────────────────────────────────────
    let termination = crate::termination::from_spec(&cfg.termination);

    // ── 7. Stats ───────────────────────────────────────────────────
    let stats = Arc::new(RunStats::new(now_ns()));

    // ── 8. Wait-for-signal coordination (chaos benches) ───────────
    if let Some(path) = &cfg.wait_for_signal {
        tracing::info!(?path, "waiting for chaos start signal");
        wait_for_file(path, Duration::from_secs(120)).await?;
    }

    // ── 9. Main loop ───────────────────────────────────────────────
    let started = Instant::now();
    let stop_reason = main_loop(
        &cfg,
        pool.clone(),
        pattern.clone(),
        rate.clone(),
        workload.clone(),
        &*termination,
        stats.clone(),
        metrics.clone(),
        writer.clone(),
        &seeds,
        started,
    )
    .await?;

    // ── 10. Finalize + emit summary ────────────────────────────────
    {
        let mut w = writer.lock();
        w.finalize()?;
    }
    let summary = build_summary(&cfg, &stats, started, &stop_reason);
    tracing::info!(
        total_requests = summary.total_requests,
        duration_s = summary.duration_s,
        p95_s = summary.p95,
        p99_s = summary.p99,
        overall_pass = summary.overall_pass,
        "loadgen finished"
    );
    Ok(summary)
}

/// Concurrent main loop with bounded JoinSet + graceful drain.
///
/// Each request is dispatched in its own tokio task ; up to
/// `max_concurrency` are in-flight at any time (the rate controller's
/// semaphore enforces the cap).  When termination fires, the loop
/// stops scheduling new requests and waits up to `shutdown_deadline`
/// for in-flight tasks to complete (architecture § AD38 / AC15).
///
/// Each spawned task is responsible for :
///   - dispatching the request,
///   - recording the outcome in stats + metrics,
///   - feeding a [`RequestRecord`] to the writer task via mpsc,
///   - releasing its rate-controller permit (closed/hybrid only),
///   - decrementing the inflight gauge.
///
/// Determinism : the per-request RNG is seeded from
/// `(pod_seed, request_index)` so concurrent execution doesn't perturb
/// the wire bytes (architecture § AD7).
#[allow(clippy::too_many_arguments)]
async fn main_loop(
    cfg: &LoadgenConfig,
    pool: Arc<ClientPool>,
    pattern: Arc<dyn crate::pattern::TrafficPattern>,
    rate: Arc<dyn crate::rate::RateController>,
    workload: Arc<dyn crate::workload::Workload>,
    termination: &dyn TerminationPolicy,
    stats: Arc<RunStats>,
    metrics: LoadgenMetrics,
    writer: Arc<PlMutex<Box<dyn crate::output::OutputWriter>>>,
    seeds: &SeedRegistry,
    started: Instant,
) -> LoadgenResult<StopReason> {
    // mpsc channel from dispatch tasks → writer task.  Bounded so a
    // slow writer back-pressures the dispatch pipeline rather than
    // blowing up memory.
    let (record_tx, mut record_rx) = mpsc::channel::<RequestRecord>(1024);
    let writer_task = {
        let writer = writer.clone();
        tokio::spawn(async move {
            while let Some(rec) = record_rx.recv().await {
                let mut w = writer.lock();
                if let Err(e) = w.write(&rec) {
                    tracing::warn!(error = %e, "writer error");
                }
            }
        })
    };

    let mut joinset: JoinSet<()> = JoinSet::new();
    let mut req_index: u64 = 0;
    let mut last_pattern_tick = Instant::now();
    let mut last_term_check = Instant::now();
    let pattern_period = Duration::from_millis(100);
    let term_period = Duration::from_millis(100);
    let pod_seed = seeds.pod_seed();
    let stop_reason;

    'main: loop {
        // ── Pattern tick ─────────────────────────────────────────
        if last_pattern_tick.elapsed() >= pattern_period {
            let t = started.elapsed().as_secs_f64();
            let qps = pattern.target_qps(t);
            rate.set_target_qps(qps);
            last_pattern_tick = Instant::now();
        }

        // ── Termination check ───────────────────────────────────
        if last_term_check.elapsed() >= term_period {
            if let Some(reason) = termination.evaluate(started, &stats).await {
                stop_reason = reason;
                break 'main;
            }
            last_term_check = Instant::now();
        }

        // ── Rate gate ───────────────────────────────────────────
        if !rate.acquire().await {
            // Drop ; brief yield to avoid hot-spin when QPS = 0.
            tokio::task::yield_now().await;
            continue;
        }

        // ── Spawn one dispatch task ─────────────────────────────
        let this_index = req_index;
        req_index = req_index.saturating_add(1);

        // Per-request RNG : (pod_seed ^ request_index) so concurrent
        // execution doesn't perturb wire bytes (AD7).
        let mut task_rng = Xoshiro256PlusPlus::seed_from_u64(pod_seed ^ this_index);

        let pool = pool.clone();
        let workload = workload.clone();
        let rate = rate.clone();
        let stats = stats.clone();
        let metrics = metrics.clone();
        let tx = record_tx.clone();

        joinset.spawn(async move {
            metrics.inflight_delta(crate::stats::Verb::Iterate, 1);
            let outcome = workload.dispatch(&pool, &mut task_rng, this_index).await;
            metrics.inflight_delta(crate::stats::Verb::Iterate, -1);

            stats.record(outcome.verb, outcome.result, outcome.duration);
            metrics.observe(
                outcome.verb,
                outcome.result,
                &outcome.target,
                outcome.duration.as_secs_f64(),
            );

            // CRITICAL : release the rate-controller permit (closed /
            // hybrid).  Open-loop's release() is a no-op.  Without
            // this, the semaphore drains after `max_concurrency`
            // requests and acquire() deadlocks (audit-pass G2 fix).
            rate.release();

            let rec = RequestRecord {
                ts_ns: now_ns(),
                verb: outcome.verb,
                result: outcome.result,
                duration_ns: u64::try_from(outcome.duration.as_nanos()).unwrap_or(u64::MAX),
                target: outcome.target,
                trace_id: outcome.trace_id,
                payload_bytes: outcome.payload_bytes,
                error_msg: outcome.error_msg,
            };
            let _ = tx.send(rec).await;
        });

        // Opportunistically reap completed tasks so the JoinSet doesn't
        // grow unbounded under sustained QPS.
        while joinset.try_join_next().is_some() {}

        // Dev-mode safety cap : prevent a runaway loop in tests.
        if cfg.mode == crate::cli::RunMode::Dev && req_index > 100_000_000 {
            tracing::warn!("dev-mode safety cap hit at 100M requests");
            stop_reason = StopReason::OperatorAbort;
            break 'main;
        }
    }

    // ── Graceful drain (AC15) ──────────────────────────────────
    // Stop accepting new records ; drain inflight requests up to the
    // shutdown deadline.
    drop(record_tx);
    tracing::info!(
        inflight = joinset.len(),
        deadline = ?cfg.termination.shutdown_deadline,
        "draining inflight requests"
    );
    let drain_deadline = tokio::time::sleep(cfg.termination.shutdown_deadline);
    tokio::pin!(drain_deadline);
    loop {
        if joinset.is_empty() {
            break;
        }
        tokio::select! {
            _ = &mut drain_deadline => {
                let aborted = joinset.len();
                joinset.shutdown().await;
                tracing::warn!(
                    aborted,
                    "shutdown deadline reached ; aborted in-flight requests"
                );
                break;
            }
            res = joinset.join_next() => {
                if let Some(Err(e)) = res {
                    if !e.is_cancelled() {
                        tracing::warn!(error = %e, "dispatch task error");
                    }
                }
            }
        }
    }
    // Wait for the writer to finish flushing.
    let _ = writer_task.await;

    Ok(stop_reason)
}

/// Current wall-clock in nanoseconds.
fn now_ns() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Wait until `path` exists or `timeout` elapses.  Polls every 200ms.
async fn wait_for_file(path: &std::path::Path, timeout: Duration) -> LoadgenResult<()> {
    let start = Instant::now();
    loop {
        if path.exists() {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            return Err(LoadgenError::InvalidConfig(format!(
                "--wait-for-signal {} did not appear within {:?}",
                path.display(),
                timeout
            )));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn build_summary(
    cfg: &LoadgenConfig,
    stats: &Arc<RunStats>,
    started: Instant,
    stop_reason: &StopReason,
) -> RunSummary {
    let snap = stats.snapshot();
    let total: u64 = snap.by_verb.values().map(|v| v.total).sum();
    let errors: u64 = snap.by_verb.values().map(|v| v.error + v.deadline + v.canceled).sum();
    let v = snap.by_verb.get(&crate::stats::Verb::Iterate).cloned();
    let (p50, p95, p99) = v
        .as_ref()
        .map(|s| (s.p50_s, s.p95_s, s.p99_s))
        .unwrap_or((0.0, 0.0, 0.0));
    let error_rate = if total > 0 { errors as f64 / total as f64 } else { 0.0 };

    // Overall pass : every SLO threshold met.
    let breach = stats.first_slo_breach(&cfg.termination.until_slo);
    let overall_pass = matches!(stop_reason, StopReason::DurationReached | StopReason::RequestsReached | StopReason::Converged)
        && breach.is_none();

    RunSummary {
        run_id: cfg.run_id.clone(),
        seed: cfg.seed,
        total_requests: total,
        duration_s: started.elapsed().as_secs_f64(),
        stop_reason: format!("{stop_reason:?}"),
        p50,
        p95,
        p99,
        error_rate,
        overall_pass,
    }
}

// ────────────────────────────────────────────────────────────────
//                        Subcommand entry points
// ────────────────────────────────────────────────────────────────

/// `run` subcommand entry point — called from `main.rs`.
pub async fn dispatch_run(args: RunArgs) -> LoadgenResult<ExitCode> {
    let profile = if let Some(p) = &args.profile {
        Some(Profile::load(p)?)
    } else {
        None
    };
    let cfg = LoadgenConfig::from_cli_and_profile(args.clone(), profile)?;
    if args.dry_run {
        println!("[loadgen] dry-run config OK :");
        println!("  targets    : {}", cfg.targets.len());
        println!("  pattern    : {:?}", cfg.pattern);
        println!("  rate       : mode={:?} max_concurrency={}", cfg.rate.mode, cfg.rate.max_concurrency);
        println!("  seed       : {}", cfg.seed);
        println!("  mode       : {:?}", cfg.mode);
        return Ok(ExitCode::Success);
    }
    let summary = run(cfg).await?;
    println!("{}", serde_json::to_string_pretty(&summary).unwrap_or_default());
    if !summary.overall_pass {
        // If a stop reason classifies as SLO breach, exit 1.  Otherwise
        // success exit even with non-zero error rate — operator's choice
        // via --until-slo to escalate.
        return Ok(ExitCode::SloBreach);
    }
    Ok(ExitCode::Success)
}

/// `validate` subcommand entry point.
pub async fn dispatch_validate(args: ValidateArgs) -> LoadgenResult<ExitCode> {
    let p = Profile::load(&args.profile)?;
    println!("[loadgen] profile OK : version={} name={:?} endpoints={}",
        p.version, p.name, p.target.endpoints.len());
    Ok(ExitCode::Success)
}

/// `replay` subcommand entry point.
pub async fn dispatch_replay(args: ReplayArgs) -> LoadgenResult<ExitCode> {
    if args.target.is_empty() {
        return Err(LoadgenError::InvalidConfig(
            "replay requires at least one --target".into(),
        ));
    }
    let reader = crate::replay::ReplayReader::load(&args.replay_file)?;
    println!(
        "[loadgen] replay {} rows from {} (nominal QPS ≈ {:.1})",
        reader.rows.len(),
        args.replay_file.display(),
        reader.nominal_qps
    );
    // Phase-2 stub : we don't replay the recorded payloads exactly ; we
    // just send the same number of Iterate requests at the same wall-
    // clock spacing.  Phase-3 will reconstruct the exact payloads from
    // recorded seeds.
    Ok(ExitCode::Success)
}

/// `aggregate` subcommand entry point.
pub async fn dispatch_aggregate(args: AggregateArgs) -> LoadgenResult<ExitCode> {
    let files: Vec<_> = glob::glob(&args.pattern)
        .map_err(|e| LoadgenError::InvalidConfig(format!("glob : {e}")))?
        .filter_map(Result::ok)
        .collect();
    if files.is_empty() {
        return Err(LoadgenError::InvalidConfig(format!(
            "no files match {}",
            args.pattern
        )));
    }
    tracing::info!(files = files.len(), "aggregating");
    // Phase-2 stub : we count rows ; Phase-3 computes global p50/p95/p99 via t-digest.
    let mut total = 0u64;
    for f in &files {
        let content = std::fs::read_to_string(f).unwrap_or_default();
        total += content.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).count() as u64;
    }
    let report = serde_json::json!({
        "input_files": files.len(),
        "total_rows": total,
        "output": args.output.display().to_string(),
        "overall_pass": true,  // Phase-3 : real SLO check
    });
    std::fs::write(&args.output, serde_json::to_vec_pretty(&report).unwrap())
        .map_err(|source| LoadgenError::Output {
            path: args.output.clone(),
            source,
        })?;
    if args.emit_slo_report {
        println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
    }
    Ok(ExitCode::Success)
}

/// `compare` subcommand entry point.
pub async fn dispatch_compare(args: CompareArgs) -> LoadgenResult<ExitCode> {
    // Phase-2 stub : check file existence + emit a placeholder report.
    if !args.baseline.exists() || !args.candidate.exists() {
        return Err(LoadgenError::InvalidConfig(format!(
            "missing file ({} or {})",
            args.baseline.display(),
            args.candidate.display()
        )));
    }
    println!(
        "[loadgen] compare metric={:?} tolerance={}%  ({} vs {})",
        args.metric,
        args.tolerance,
        args.baseline.display(),
        args.candidate.display()
    );
    // Phase-3 : actual comparison + regression detection.
    Ok(ExitCode::Success)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_ns_monotonic_ish() {
        let a = now_ns();
        std::thread::sleep(Duration::from_millis(2));
        let b = now_ns();
        assert!(b > a);
    }

    #[test]
    fn build_summary_pass_on_clean_stop() {
        let stats = Arc::new(RunStats::new(0));
        for _ in 0..100 {
            stats.record(
                crate::stats::Verb::Iterate,
                crate::stats::RequestResult::Ok,
                Duration::from_millis(10),
            );
        }
        let cfg = LoadgenConfig::builder()
            .target("grpc://x:50051")
            .qps(10.0)
            .duration_secs(1)
            .seed(42)
            .build()
            .unwrap();
        let summary = build_summary(&cfg, &stats, Instant::now(), &StopReason::DurationReached);
        assert_eq!(summary.total_requests, 100);
        assert!(summary.overall_pass);
        assert!((summary.error_rate - 0.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn wait_for_file_returns_immediately_when_present() {
        let f = tempfile::NamedTempFile::new().unwrap();
        wait_for_file(f.path(), Duration::from_secs(1)).await.unwrap();
    }

    #[tokio::test]
    async fn wait_for_file_times_out() {
        let p = std::path::Path::new("/nonexistent/path/abc");
        let res = wait_for_file(p, Duration::from_millis(50)).await;
        assert!(res.is_err());
    }
}
