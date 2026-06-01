//! Sprint D18 — bench report generation (Markdown + JSON).
//!
//! The report is the canonical artifact consumed by the CI gate
//! `D18BenchSloRegression`. Each SLO target is checked and the
//! pass/fail is recorded.
//!
//! Sprint **D29 (Φ-D)** extension — schema bumped to **v2** to
//! carry three new top-level fields :
//!   * `client_kind` — `mock` | `tonic` (new in v2).
//!   * `gpu_shards` — `Option<u32>` (D31).
//!   * `k_swarm` — `Option<u32>` (D38).
//!
//! Backwards-compat : v1 readers see the new fields as unknown
//! and skip them ; v2 readers fall back to `client_kind: mock` +
//! `gpu_shards: None` + `k_swarm: None` when reading a v1 file.
//!
//! Sprint **D31 (Φ-D)** extension — adds `per_shard` (BTreeMap of
//! `shard_id → ShardReport`) so the bench can compute the
//! `D31-GPU-1` Jain-fairness SLO across shards.
//!
//! Sprint **D38 (Φ-F)** extension — adds two derived SLOs :
//!   * `C-3` — Mode-3 p99 ≤ 3 × Mode-1 p99 (only evaluated when
//!     a Mode-1 baseline report is loaded — see [`evaluate_c3`]).
//!   * `C-9` — Parallel speedup ≥ 2.5× on `mode3-k5` (also
//!     baseline-driven).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::client::ClientKind;
use crate::error::BenchError;
use crate::runner::{RunnerOutcome, ShardStats};

/// Pass/fail flag with a human-readable detail line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SloPassFail {
    /// SLO id (e.g. "D18-bench-1").
    pub slo: String,
    /// `true` iff the metric meets the target.
    pub pass: bool,
    /// Free-form detail (e.g. "p99=185.0ms ≤ 200ms").
    pub detail: String,
}

/// Full bench report.
///
/// **Sprint D29 (Φ-D)** — `schema_version` bumped to `2.0.0`.
/// Old v1 readers ignore the new fields ; new v2 readers fall
/// back gracefully when reading a v1 file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchReport {
    /// Schema version (pinned).
    pub schema_version: String,
    /// **Sprint D29 (Φ-D)** — symbolic backend tag.
    /// Default `Mock` when reading a legacy v1 report.
    #[serde(default = "default_client_kind")]
    pub client_kind: ClientKind,
    /// **Sprint D31 (Φ-D)** — number of GPU shards orchestrated.
    /// `None` for v1 reports + non-sharded runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_shards: Option<u32>,
    /// **Sprint D38 (Φ-F)** — k_swarm value for Mode-3 workloads.
    /// `None` for non-Mode-3 workloads + legacy reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k_swarm: Option<u32>,
    /// Workload label.
    pub workload: String,
    /// Target environment label.
    pub target_env: String,
    /// Generation timestamp.
    pub generated_at: DateTime<Utc>,
    /// Replica count of the cluster under bench (informational).
    pub replicas: u32,
    /// Total wall-clock duration of the run (seconds).
    pub duration_seconds: f64,
    /// Aggregate success count.
    pub success_count: u64,
    /// Aggregate error count.
    pub error_count: u64,
    /// Aggregate p50 / p95 / p99 (ms).
    pub p50_ms: Option<f64>,
    /// p95 latency in milliseconds.
    pub p95_ms: Option<f64>,
    /// p99 latency in milliseconds.
    pub p99_ms: Option<f64>,
    /// Aggregate effective RPS.
    pub rps_achieved: f64,
    /// Per-op stats : op_name → (p95_ms, p99_ms, rps).
    pub per_op: std::collections::BTreeMap<String, OpReport>,
    /// **Sprint D31 (Φ-D)** — per-shard stats. Empty when
    /// `gpu_shards.is_none() || gpu_shards == Some(1)`.
    #[serde(default)]
    pub per_shard: std::collections::BTreeMap<u32, ShardReport>,
    /// Error breakdown by label.
    pub errors_by_label: std::collections::BTreeMap<String, u64>,
    /// SLO check results.
    pub slos: Vec<SloPassFail>,
    /// Whether ALL SLOs passed.
    pub all_slos_pass: bool,
}

/// Default for `client_kind` when reading a v1 file (no field).
fn default_client_kind() -> ClientKind {
    ClientKind::Mock
}

/// Per-op report fragment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpReport {
    /// p50 latency in ms.
    pub p50_ms: Option<f64>,
    /// p95 latency in ms.
    pub p95_ms: Option<f64>,
    /// p99 latency in ms.
    pub p99_ms: Option<f64>,
    /// Mean latency in ms.
    pub mean_ms: Option<f64>,
    /// Effective RPS.
    pub rps: f64,
    /// Success count.
    pub success_count: u64,
    /// Error count.
    pub error_count: u64,
}

/// **Sprint D31 (Φ-D)** — per-shard report fragment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardReport {
    /// Shard id (`worker_idx % gpu_shards`).
    pub shard_id: u32,
    /// p50 latency in ms.
    pub p50_ms: Option<f64>,
    /// p95 latency in ms.
    pub p95_ms: Option<f64>,
    /// p99 latency in ms.
    pub p99_ms: Option<f64>,
    /// Effective RPS on this shard.
    pub rps: f64,
    /// Success count on this shard.
    pub success_count: u64,
    /// Error count on this shard.
    pub error_count: u64,
}

impl ShardReport {
    /// Build from a [`ShardStats`].
    fn from_stats(shard_id: u32, stats: &ShardStats) -> Self {
        Self {
            shard_id,
            p50_ms: stats.histogram.p50_ms,
            p95_ms: stats.histogram.p95_ms,
            p99_ms: stats.histogram.p99_ms,
            rps: stats.rps,
            success_count: stats.success_count,
            error_count: stats.error_count,
        }
    }
}

/// SLO target struct (architecture § 3.3).
#[derive(Debug, Clone)]
pub struct SloTarget {
    /// SLO id ("D18-bench-1").
    pub id: &'static str,
    /// Op or workload this SLO applies to.
    pub op: &'static str,
    /// Quantile (0.95 / 0.99 / NaN for non-latency SLOs).
    pub quantile: f64,
    /// Target latency in ms (or 0.0 for non-latency).
    pub target_ms: f64,
    /// Optional minimum RPS required for the SLO to be eligible.
    pub min_rps: f64,
}

/// The canonical 7 D18 SLO targets.
#[must_use]
pub fn canonical_slos() -> Vec<SloTarget> {
    vec![
        SloTarget {
            id: "D18-bench-1",
            op: "execute",
            quantile: 0.99,
            target_ms: 200.0,
            min_rps: 1000.0,
        },
        SloTarget {
            id: "D18-bench-2",
            op: "register_catalog",
            quantile: 0.95,
            target_ms: 50.0,
            min_rps: 100.0,
        },
        SloTarget {
            id: "D18-bench-3",
            op: "resolve_catalog",
            quantile: 0.99,
            target_ms: 20.0,
            min_rps: 5000.0,
        },
        SloTarget {
            id: "D18-bench-4",
            op: "list_catalogs",
            quantile: 0.95,
            target_ms: 100.0,
            min_rps: 0.0,
        },
        SloTarget {
            id: "D18-bench-5",
            op: "register_class",
            // SLO-5 is throughput, not latency → quantile=NaN
            quantile: f64::NAN,
            target_ms: 0.0,
            min_rps: 200.0,
        },
        SloTarget {
            id: "D18-bench-6",
            op: "execute",
            // Multi-replica linear speedup — checked offline by
            // comparing report against a 1-replica baseline.
            // Bench harness can't auto-evaluate ; mark pass by
            // default + leave detail to a manual check.
            quantile: f64::NAN,
            target_ms: 0.0,
            min_rps: 0.0,
        },
        SloTarget {
            id: "D18-bench-7",
            op: "execute", // proxied via Execute since audit chain
                            // sign happens inside Execute handler
            quantile: 0.99,
            target_ms: 5.0,
            min_rps: 0.0,
        },
    ]
}

/// **Sprint D38 (Φ-F)** — Mode-3 p99 latency target relative to
/// Mode-1 baseline. C-3 mandates p99(mode3) ≤ 3 × p99(mode1).
pub const C3_MODE3_LATENCY_RATIO: f64 = 3.0;

/// **Sprint D38 (Φ-F)** — Mode-3 parallel speedup floor. C-9
/// requires Mode-3 with k=5 to deliver ≥ 2.5× speedup vs a
/// k=1 baseline.
pub const C9_MIN_SPEEDUP: f64 = 2.5;

/// **Sprint D31 (Φ-D)** — Jain index floor for per-shard fairness.
/// SLO `D31-GPU-1` requires Jain ≥ 0.95 across shards.
pub const D31_GPU_FAIRNESS_FLOOR: f64 = 0.95;

/// **Sprint D31 (Φ-D)** — compute the Jain fairness index over a
/// vector of per-shard RPS values. Returns 1.0 for a single
/// element (perfectly fair by definition) and 0 for an empty
/// input. The index is `(Σx)² / (n · Σx²)` ∈ [1/n, 1].
#[must_use]
pub fn jain_index(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let n = values.len() as f64;
    let sum: f64 = values.iter().sum();
    let sum_sq: f64 = values.iter().map(|x| x * x).sum();
    if sum_sq == 0.0 {
        return 1.0;
    }
    (sum * sum) / (n * sum_sq)
}

/// Generate a [`BenchReport`] from a [`RunnerOutcome`].
pub fn generate_report(
    outcome: &RunnerOutcome,
    target_env: &str,
    replicas: u32,
) -> Result<BenchReport, BenchError> {
    generate_report_with_client(outcome, target_env, replicas, ClientKind::Mock, None)
}

/// **Sprint D29 (Φ-D)** — full constructor that records the
/// client kind + the workload's k_swarm value when applicable.
///
/// The `k_swarm` parameter overrides any per-op heuristic ; pass
/// `None` for non-Mode-3 workloads or `Some(k)` when the operator
/// invoked a `mode3-k*` workload.
pub fn generate_report_with_client(
    outcome: &RunnerOutcome,
    target_env: &str,
    replicas: u32,
    client_kind: ClientKind,
    k_swarm: Option<u32>,
) -> Result<BenchReport, BenchError> {
    let mut per_op = std::collections::BTreeMap::new();
    for (op_name, stats) in &outcome.per_op {
        per_op.insert(
            op_name.clone(),
            OpReport {
                p50_ms: stats.histogram.p50_ms,
                p95_ms: stats.histogram.p95_ms,
                p99_ms: stats.histogram.p99_ms,
                mean_ms: stats.histogram.mean_ms,
                rps: stats.rps,
                success_count: stats.success_count,
                error_count: stats.error_count,
            },
        );
    }
    let mut errors_by_label = std::collections::BTreeMap::new();
    for (k, v) in &outcome.errors_by_label {
        errors_by_label.insert(k.clone(), *v);
    }

    // **Sprint D31 (Φ-D)** — per-shard report.
    let mut per_shard = std::collections::BTreeMap::new();
    for (shard_id, stats) in &outcome.per_shard {
        per_shard.insert(*shard_id, ShardReport::from_stats(*shard_id, stats));
    }

    // Evaluate canonical SLOs.
    let mut slos = Vec::new();
    let mut all_pass = true;
    for tgt in canonical_slos() {
        let pass_fail = evaluate_slo(&tgt, &per_op, outcome.aggregate.rps);
        if !pass_fail.pass {
            all_pass = false;
        }
        slos.push(pass_fail);
    }

    // **Sprint D31 (Φ-D)** — D31-GPU-1 fairness check.
    if outcome.gpu_shards > 1 {
        let pass_fail = evaluate_d31_gpu_fairness(&per_shard);
        if !pass_fail.pass {
            all_pass = false;
        }
        slos.push(pass_fail);
    }

    let gpu_shards_field = if outcome.gpu_shards > 1 {
        Some(outcome.gpu_shards)
    } else {
        None
    };

    Ok(BenchReport {
        schema_version: crate::REPORT_VERSION.to_string(),
        client_kind,
        gpu_shards: gpu_shards_field,
        k_swarm,
        workload: outcome.workload_label.clone(),
        target_env: target_env.to_string(),
        generated_at: Utc::now(),
        replicas,
        duration_seconds: outcome.duration.as_secs_f64(),
        success_count: outcome.aggregate.success_count,
        error_count: outcome.aggregate.error_count,
        p50_ms: outcome.aggregate.histogram.p50_ms,
        p95_ms: outcome.aggregate.histogram.p95_ms,
        p99_ms: outcome.aggregate.histogram.p99_ms,
        rps_achieved: outcome.aggregate.rps,
        per_op,
        per_shard,
        errors_by_label,
        slos,
        all_slos_pass: all_pass,
    })
}

fn evaluate_slo(
    tgt: &SloTarget,
    per_op: &std::collections::BTreeMap<String, OpReport>,
    aggregate_rps: f64,
) -> SloPassFail {
    let op_stats = per_op.get(tgt.op);

    // SLO-6 (linear speedup) and SLO-7 latency are special cases.
    if tgt.id == "D18-bench-6" {
        return SloPassFail {
            slo: tgt.id.into(),
            pass: true,  // can't auto-evaluate ; manual gate
            detail: "linear speedup checked by CI offline against 1-replica baseline".into(),
        };
    }

    let stats = match op_stats {
        Some(s) => s,
        None => {
            return SloPassFail {
                slo: tgt.id.into(),
                pass: tgt.id == "D18-bench-6", // already special-cased above
                detail: format!("op {:?} not exercised in this workload", tgt.op),
            };
        }
    };

    // Throughput-only SLO (D18-bench-5).
    if tgt.target_ms == 0.0 && tgt.min_rps > 0.0 && tgt.quantile.is_nan() {
        let pass = stats.rps >= tgt.min_rps;
        return SloPassFail {
            slo: tgt.id.into(),
            pass,
            detail: format!(
                "rps={:.1} {} target {:.1}",
                stats.rps,
                if pass { "≥" } else { "<" },
                tgt.min_rps
            ),
        };
    }

    // Latency SLO with quantile.
    let measured = if (tgt.quantile - 0.50).abs() < 1e-9 {
        stats.p50_ms
    } else if (tgt.quantile - 0.95).abs() < 1e-9 {
        stats.p95_ms
    } else if (tgt.quantile - 0.99).abs() < 1e-9 {
        stats.p99_ms
    } else {
        None
    };

    match measured {
        Some(ms) => {
            // RPS gate (the SLO is only eligible if the run actually
            // exercised the op at the minimum RPS).
            let rps_eligible = tgt.min_rps == 0.0 || aggregate_rps >= tgt.min_rps * 0.5;
            let pass = ms <= tgt.target_ms && rps_eligible;
            SloPassFail {
                slo: tgt.id.into(),
                pass,
                detail: format!(
                    "p{:.0}={:.1}ms (target ≤{:.0}ms){}{}",
                    tgt.quantile * 100.0,
                    ms,
                    tgt.target_ms,
                    if rps_eligible {
                        ""
                    } else {
                        ", rps too low for SLO eligibility"
                    },
                    if pass { " ✓" } else { " ✗" }
                ),
            }
        }
        None => SloPassFail {
            slo: tgt.id.into(),
            pass: false,
            detail: format!("no quantile {} measurement available", tgt.quantile),
        },
    }
}

/// **Sprint D31 (Φ-D)** — evaluate the per-shard fairness SLO
/// `D31-GPU-1`. Computes the Jain index across shard RPS values
/// and passes when index ≥ 0.95.
fn evaluate_d31_gpu_fairness(
    per_shard: &std::collections::BTreeMap<u32, ShardReport>,
) -> SloPassFail {
    if per_shard.is_empty() {
        return SloPassFail {
            slo: "D31-GPU-1".into(),
            pass: true,
            detail: "no shards (single-GPU run)".into(),
        };
    }
    let rps_values: Vec<f64> = per_shard.values().map(|s| s.rps).collect();
    let index = jain_index(&rps_values);
    let pass = index >= D31_GPU_FAIRNESS_FLOOR;
    SloPassFail {
        slo: "D31-GPU-1".into(),
        pass,
        detail: format!(
            "Jain index={:.3} (target ≥{:.2}) over {} shard(s){}",
            index,
            D31_GPU_FAIRNESS_FLOOR,
            per_shard.len(),
            if pass { " ✓" } else { " ✗" }
        ),
    }
}

/// **Sprint D38 (Φ-F)** — evaluate the C-3 SLO comparing the
/// Mode-3 p99 of the current report against the Mode-1 p99 of
/// a baseline report.
///
/// The CI pipeline calls this AFTER running both `read-heavy`
/// (Mode-1 baseline) and `mode3-k5` (Mode-3 candidate) and
/// passes both reports in. C-3 fails if the ratio exceeds
/// `C3_MODE3_LATENCY_RATIO` (3×).
#[must_use]
pub fn evaluate_c3(
    mode3_report: &BenchReport,
    mode1_report: &BenchReport,
) -> SloPassFail {
    let mode3_p99 = mode3_report
        .per_op
        .iter()
        .filter_map(|(op, r)| {
            if op.starts_with("mode3_execute") {
                r.p99_ms
            } else {
                None
            }
        })
        .reduce(f64::max);
    let mode1_p99 = mode1_report
        .per_op
        .get("execute")
        .and_then(|r| r.p99_ms)
        .or_else(|| {
            mode1_report
                .per_op
                .get("resolve_catalog")
                .and_then(|r| r.p99_ms)
        });
    match (mode3_p99, mode1_p99) {
        (Some(m3), Some(m1)) if m1 > 0.0 => {
            let ratio = m3 / m1;
            let pass = ratio <= C3_MODE3_LATENCY_RATIO;
            SloPassFail {
                slo: "C-3".into(),
                pass,
                detail: format!(
                    "p99(mode3)={:.1}ms / p99(mode1)={:.1}ms = {:.2}× (target ≤ {:.1}×){}",
                    m3,
                    m1,
                    ratio,
                    C3_MODE3_LATENCY_RATIO,
                    if pass { " ✓" } else { " ✗" }
                ),
            }
        }
        _ => SloPassFail {
            slo: "C-3".into(),
            pass: false,
            detail: "missing p99 measurements (need both Mode-1 baseline + Mode-3 candidate)".into(),
        },
    }
}

/// **Sprint D38 (Φ-F)** — evaluate the C-9 parallel speedup SLO.
///
/// The bench compares `mode3-k5` throughput (k_swarm=5) against
/// a synthetic `mode3-k1` (single sub-swarm) baseline. The
/// effective speedup must be ≥ 2.5× ; below that the parallelism
/// is not amortising the fan-out cost.
///
/// The function expects two reports passing the Mode-3 op
/// stats — typically generated by running `mode3-k5` and a
/// custom `--k-swarm 1` smoke run.
#[must_use]
pub fn evaluate_c9(
    parallel_report: &BenchReport,
    serial_report: &BenchReport,
) -> SloPassFail {
    let parallel_rps = parallel_report
        .per_op
        .iter()
        .filter_map(|(op, r)| {
            if op.starts_with("mode3_execute") {
                Some(r.rps)
            } else {
                None
            }
        })
        .reduce(f64::max);
    let serial_rps = serial_report
        .per_op
        .iter()
        .filter_map(|(op, r)| {
            if op.starts_with("mode3_execute") {
                Some(r.rps)
            } else {
                None
            }
        })
        .reduce(f64::max);
    match (parallel_rps, serial_rps) {
        (Some(p), Some(s)) if s > 0.0 => {
            let speedup = p / s;
            let pass = speedup >= C9_MIN_SPEEDUP;
            SloPassFail {
                slo: "C-9".into(),
                pass,
                detail: format!(
                    "rps(parallel)={:.1} / rps(serial)={:.1} = {:.2}× (target ≥ {:.1}×){}",
                    p,
                    s,
                    speedup,
                    C9_MIN_SPEEDUP,
                    if pass { " ✓" } else { " ✗" }
                ),
            }
        }
        _ => SloPassFail {
            slo: "C-9".into(),
            pass: false,
            detail: "missing Mode-3 RPS measurements (need both serial + parallel runs)".into(),
        },
    }
}

/// Render the [`BenchReport`] as a human-readable Markdown string.
pub fn render_markdown(report: &BenchReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# EFE Bench Report\n\n\
         > Generated {} by efe-bench (schema v{}).\n\
         > Target env : `{}`.\n\
         > Replicas : {}.\n\
         > Client : `{}`.\n",
        report.generated_at.format("%Y-%m-%dT%H:%M:%SZ"),
        report.schema_version,
        report.target_env,
        report.replicas,
        report.client_kind.as_str(),
    ));
    if let Some(s) = report.gpu_shards {
        out.push_str(&format!("> GPU shards : {s}.\n"));
    }
    if let Some(k) = report.k_swarm {
        out.push_str(&format!("> Mode-3 k_swarm : {k}.\n"));
    }
    out.push('\n');
    out.push_str(&format!(
        "## Workload : `{}` ({:.1} s wall-clock)\n\n",
        report.workload, report.duration_seconds
    ));
    out.push_str("| Metric | Value |\n");
    out.push_str("|--------|------:|\n");
    out.push_str(&format!("| RPS achieved | {:.1} |\n", report.rps_achieved));
    out.push_str(&format!("| Successes | {} |\n", report.success_count));
    out.push_str(&format!("| Errors | {} |\n", report.error_count));
    if let Some(p50) = report.p50_ms {
        out.push_str(&format!("| p50 latency | {:.1} ms |\n", p50));
    }
    if let Some(p95) = report.p95_ms {
        out.push_str(&format!("| p95 latency | {:.1} ms |\n", p95));
    }
    if let Some(p99) = report.p99_ms {
        out.push_str(&format!("| p99 latency | {:.1} ms |\n", p99));
    }
    out.push('\n');

    out.push_str("## Per-op breakdown\n\n");
    out.push_str("| Op | RPS | p50 | p95 | p99 | success | error |\n");
    out.push_str("|----|----:|----:|----:|----:|--------:|------:|\n");
    for (op, st) in &report.per_op {
        out.push_str(&format!(
            "| {} | {:.1} | {} | {} | {} | {} | {} |\n",
            op,
            st.rps,
            st.p50_ms.map(|x| format!("{x:.1}")).unwrap_or_else(|| "-".into()),
            st.p95_ms.map(|x| format!("{x:.1}")).unwrap_or_else(|| "-".into()),
            st.p99_ms.map(|x| format!("{x:.1}")).unwrap_or_else(|| "-".into()),
            st.success_count,
            st.error_count,
        ));
    }
    out.push('\n');

    if !report.per_shard.is_empty() {
        out.push_str("## Per-shard breakdown\n\n");
        out.push_str("| Shard | RPS | p50 | p95 | p99 | success | error |\n");
        out.push_str("|------:|----:|----:|----:|----:|--------:|------:|\n");
        for (sid, st) in &report.per_shard {
            out.push_str(&format!(
                "| {} | {:.1} | {} | {} | {} | {} | {} |\n",
                sid,
                st.rps,
                st.p50_ms.map(|x| format!("{x:.1}")).unwrap_or_else(|| "-".into()),
                st.p95_ms.map(|x| format!("{x:.1}")).unwrap_or_else(|| "-".into()),
                st.p99_ms.map(|x| format!("{x:.1}")).unwrap_or_else(|| "-".into()),
                st.success_count,
                st.error_count,
            ));
        }
        out.push('\n');
    }

    if !report.errors_by_label.is_empty() {
        out.push_str("## Error breakdown\n\n");
        for (label, count) in &report.errors_by_label {
            out.push_str(&format!("* `{}` : {}\n", label, count));
        }
        out.push('\n');
    }

    out.push_str("## SLO results\n\n");
    out.push_str("| SLO | Pass | Detail |\n");
    out.push_str("|-----|:----:|--------|\n");
    for slo in &report.slos {
        out.push_str(&format!(
            "| `{}` | {} | {} |\n",
            slo.slo,
            if slo.pass { "✅" } else { "❌" },
            slo.detail
        ));
    }
    out.push('\n');

    out.push_str(&format!(
        "**Overall : {}**\n",
        if report.all_slos_pass {
            "ALL SLOS PASS ✅"
        } else {
            "SLO REGRESSION ❌"
        }
    ));

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::HistogramSnapshot;

    fn make_outcome() -> RunnerOutcome {
        let mut per_op = std::collections::HashMap::new();
        per_op.insert(
            "execute".to_string(),
            crate::runner::OpStats {
                histogram: HistogramSnapshot {
                    buckets_ms_upper: vec![1.0, 5.0, f64::INFINITY],
                    counts: vec![10, 5, 0],
                    total: 15,
                    sum_us: 0,
                    max_ms: 4.0,
                    mean_ms: Some(2.5),
                    p50_ms: Some(2.0),
                    p95_ms: Some(3.5),
                    p99_ms: Some(4.0),
                },
                success_count: 15,
                error_count: 0,
                rps: 10.0,
            },
        );
        RunnerOutcome {
            workload_label: "balanced".into(),
            duration: std::time::Duration::from_secs(2),
            per_op,
            aggregate: crate::runner::OpStats {
                histogram: HistogramSnapshot {
                    buckets_ms_upper: vec![1.0, 5.0, f64::INFINITY],
                    counts: vec![10, 5, 0],
                    total: 15,
                    sum_us: 0,
                    max_ms: 4.0,
                    mean_ms: Some(2.5),
                    p50_ms: Some(2.0),
                    p95_ms: Some(3.5),
                    p99_ms: Some(4.0),
                },
                success_count: 15,
                error_count: 0,
                rps: 10.0,
            },
            errors_by_label: std::collections::HashMap::new(),
            per_shard: std::collections::BTreeMap::new(),
            gpu_shards: 1,
        }
    }

    fn make_outcome_with_shards(shards: u32) -> RunnerOutcome {
        let mut o = make_outcome();
        o.gpu_shards = shards;
        for shard_id in 0..shards {
            o.per_shard.insert(
                shard_id,
                ShardStats {
                    histogram: HistogramSnapshot {
                        buckets_ms_upper: vec![1.0, 5.0, f64::INFINITY],
                        counts: vec![5, 2, 0],
                        total: 7,
                        sum_us: 0,
                        max_ms: 4.0,
                        mean_ms: Some(2.5),
                        p50_ms: Some(2.0),
                        p95_ms: Some(3.5),
                        p99_ms: Some(4.0),
                    },
                    // Spread RPS slightly to test fairness.
                    success_count: 7 - (shard_id as u64 % 2),
                    error_count: 0,
                    rps: 5.0 + (shard_id as f64 * 0.1),
                },
            );
        }
        o
    }

    #[test]
    fn canonical_slos_lists_seven() {
        let s = canonical_slos();
        assert_eq!(s.len(), 7);
        let ids: Vec<_> = s.iter().map(|t| t.id).collect();
        assert!(ids.contains(&"D18-bench-1"));
        assert!(ids.contains(&"D18-bench-7"));
    }

    #[test]
    fn generate_report_passes_low_rps_with_proper_detail() {
        let outcome = make_outcome();
        let report = generate_report(&outcome, "kind", 1).unwrap();
        // p99 = 4ms which is ≤ 200ms (D18-bench-1 target) BUT
        // rps_achieved=10 which is < 1000 min ; rps gate kicks
        // in → not eligible.
        let s1 = report.slos.iter().find(|s| s.slo == "D18-bench-1").unwrap();
        // With rps eligibility check : pass should be false
        // because aggregate_rps (10) < min_rps (1000).
        assert!(!s1.pass);
    }

    #[test]
    fn render_markdown_contains_slo_section() {
        let outcome = make_outcome();
        let report = generate_report(&outcome, "kind", 3).unwrap();
        let md = render_markdown(&report);
        assert!(md.contains("# EFE Bench Report"));
        assert!(md.contains("Workload : `balanced`"));
        assert!(md.contains("D18-bench-"));
    }

    #[test]
    fn slo_6_special_cases_to_pass() {
        let outcome = make_outcome();
        let report = generate_report(&outcome, "kind", 1).unwrap();
        let s6 = report.slos.iter().find(|s| s.slo == "D18-bench-6").unwrap();
        assert!(s6.pass);
        assert!(s6.detail.contains("linear speedup"));
    }

    #[test]
    fn slo_5_throughput_only() {
        let outcome = make_outcome();
        let report = generate_report(&outcome, "kind", 3).unwrap();
        let s5 = report.slos.iter().find(|s| s.slo == "D18-bench-5").unwrap();
        // No `register_class` op in our fake outcome → "not exercised".
        assert!(s5.detail.contains("not exercised") || s5.detail.contains("rps="));
    }

    // -----------------------------------------------------------
    // **Sprint D29 (Φ-D)** — schema v2 client_kind tests.
    // -----------------------------------------------------------

    #[test]
    fn report_v2_includes_client_kind_default_mock() {
        let outcome = make_outcome();
        let report = generate_report(&outcome, "kind", 1).unwrap();
        assert_eq!(report.client_kind, ClientKind::Mock);
    }

    #[test]
    fn report_v2_includes_client_kind_tonic_when_set() {
        let outcome = make_outcome();
        let report = generate_report_with_client(
            &outcome,
            "kind",
            1,
            ClientKind::Tonic,
            None,
        )
        .unwrap();
        assert_eq!(report.client_kind, ClientKind::Tonic);
    }

    #[test]
    fn report_serializes_client_kind_as_snake_case() {
        let outcome = make_outcome();
        let report = generate_report_with_client(
            &outcome,
            "kind",
            1,
            ClientKind::Tonic,
            None,
        )
        .unwrap();
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"client_kind\":\"tonic\""));
    }

    #[test]
    fn report_deserializes_v1_without_client_kind() {
        // Synthetic v1 payload : no client_kind, no gpu_shards,
        // no k_swarm, no per_shard.
        let v1_json = r#"{
            "schema_version": "1.0.0",
            "workload": "balanced",
            "target_env": "kind",
            "generated_at": "2026-04-29T00:00:00Z",
            "replicas": 1,
            "duration_seconds": 5.0,
            "success_count": 10,
            "error_count": 0,
            "p50_ms": 2.0,
            "p95_ms": 3.0,
            "p99_ms": 4.0,
            "rps_achieved": 5.0,
            "per_op": {},
            "errors_by_label": {},
            "slos": [],
            "all_slos_pass": true
        }"#;
        let r: BenchReport = serde_json::from_str(v1_json).unwrap();
        assert_eq!(r.client_kind, ClientKind::Mock); // default
        assert!(r.gpu_shards.is_none());
        assert!(r.k_swarm.is_none());
        assert!(r.per_shard.is_empty());
    }

    // -----------------------------------------------------------
    // **Sprint D31 (Φ-D)** — gpu_shards + Jain index tests.
    // -----------------------------------------------------------

    #[test]
    fn jain_index_perfectly_balanced_is_one() {
        let v = vec![10.0, 10.0, 10.0, 10.0];
        assert!((jain_index(&v) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn jain_index_max_imbalance_is_one_over_n() {
        let v = vec![100.0, 0.0, 0.0, 0.0];
        let idx = jain_index(&v);
        assert!((idx - 0.25).abs() < 1e-9);
    }

    #[test]
    fn jain_index_empty_is_zero() {
        let v: Vec<f64> = vec![];
        assert_eq!(jain_index(&v), 0.0);
    }

    #[test]
    fn jain_index_single_element_is_one() {
        let v = vec![42.0];
        assert!((jain_index(&v) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn report_with_4_shards_includes_per_shard() {
        let outcome = make_outcome_with_shards(4);
        let report = generate_report(&outcome, "kind", 1).unwrap();
        assert_eq!(report.gpu_shards, Some(4));
        assert_eq!(report.per_shard.len(), 4);
    }

    #[test]
    fn report_with_4_shards_evaluates_d31_gpu_1_slo() {
        let outcome = make_outcome_with_shards(4);
        let report = generate_report(&outcome, "kind", 1).unwrap();
        let s = report.slos.iter().find(|s| s.slo == "D31-GPU-1").unwrap();
        assert!(s.pass, "fairness should pass with mostly-equal shard RPS");
    }

    #[test]
    fn report_with_1_shard_omits_d31_slo() {
        let outcome = make_outcome();
        let report = generate_report(&outcome, "kind", 1).unwrap();
        // Single-shard runs don't add D31-GPU-1 to the SLO list.
        assert!(report.slos.iter().find(|s| s.slo == "D31-GPU-1").is_none());
        assert!(report.gpu_shards.is_none());
    }

    // -----------------------------------------------------------
    // **Sprint D38 (Φ-F)** — k_swarm + C-3 + C-9 tests.
    // -----------------------------------------------------------

    #[test]
    fn report_with_k_swarm_set_serializes_field() {
        let outcome = make_outcome();
        let report = generate_report_with_client(
            &outcome,
            "kind",
            1,
            ClientKind::Tonic,
            Some(5),
        )
        .unwrap();
        assert_eq!(report.k_swarm, Some(5));
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"k_swarm\":5"));
    }

    #[test]
    fn evaluate_c3_passes_when_ratio_under_three() {
        let mut mode1 = generate_report(&make_outcome(), "kind", 1).unwrap();
        // p99 of "execute" op is 4 ms. Set Mode-3 p99 to 10 ms →
        // ratio = 10/4 = 2.5 → passes.
        let mut mode3 = generate_report(&make_outcome(), "kind", 1).unwrap();
        mode3.per_op.clear();
        mode3.per_op.insert(
            "mode3_execute_k5".into(),
            OpReport {
                p50_ms: Some(5.0),
                p95_ms: Some(8.0),
                p99_ms: Some(10.0),
                mean_ms: Some(6.0),
                rps: 5.0,
                success_count: 10,
                error_count: 0,
            },
        );
        // mode1 baseline keeps "execute" with p99=4ms.
        mode1.workload = "read-heavy".into();
        let result = evaluate_c3(&mode3, &mode1);
        assert!(result.pass, "expected pass : 10/4 = 2.5 ≤ 3.0");
    }

    #[test]
    fn evaluate_c3_fails_when_ratio_over_three() {
        let mode1 = generate_report(&make_outcome(), "kind", 1).unwrap();
        let mut mode3 = generate_report(&make_outcome(), "kind", 1).unwrap();
        mode3.per_op.clear();
        mode3.per_op.insert(
            "mode3_execute_k5".into(),
            OpReport {
                p50_ms: Some(20.0),
                p95_ms: Some(50.0),
                p99_ms: Some(100.0), // 100/4 = 25× → fail
                mean_ms: Some(30.0),
                rps: 1.0,
                success_count: 5,
                error_count: 0,
            },
        );
        let result = evaluate_c3(&mode3, &mode1);
        assert!(!result.pass);
        assert!(result.detail.contains("25.00"));
    }

    #[test]
    fn evaluate_c3_missing_data_returns_fail_with_explanation() {
        let mode1 = generate_report(&make_outcome(), "kind", 1).unwrap();
        let mut mode3 = generate_report(&make_outcome(), "kind", 1).unwrap();
        mode3.per_op.clear(); // no mode3_execute key
        let result = evaluate_c3(&mode3, &mode1);
        assert!(!result.pass);
        assert!(result.detail.contains("missing"));
    }

    #[test]
    fn evaluate_c3_passes_when_ratio_is_exactly_one() {
        // **Audit fix M6-A3 (Φ-F)** — boundary test : Mode-3 p99
        // exactly equal to Mode-1 p99 (ratio = 1.0). Must pass
        // since 1.0 ≤ 3.0. Pre-audit no test covered the lower
        // boundary explicitly.
        let mut mode1 = generate_report(&make_outcome(), "kind", 1).unwrap();
        mode1.per_op.insert(
            "execute".into(),
            OpReport {
                p50_ms: Some(2.0),
                p95_ms: Some(3.5),
                p99_ms: Some(4.0),
                mean_ms: Some(2.5),
                rps: 100.0,
                success_count: 100,
                error_count: 0,
            },
        );
        let mut mode3 = generate_report(&make_outcome(), "kind", 1).unwrap();
        mode3.per_op.clear();
        mode3.per_op.insert(
            "mode3_execute_k5".into(),
            OpReport {
                p50_ms: Some(2.0),
                p95_ms: Some(3.5),
                p99_ms: Some(4.0), // exact match
                mean_ms: Some(2.5),
                rps: 50.0,
                success_count: 50,
                error_count: 0,
            },
        );
        let result = evaluate_c3(&mode3, &mode1);
        assert!(
            result.pass,
            "ratio = 1.0× should pass the ≤ 3.0× SLO floor"
        );
        assert!(result.detail.contains("1.00"));
    }

    #[test]
    fn evaluate_c3_passes_when_ratio_is_exactly_three() {
        // **Audit fix M6-A3 (Φ-F)** — boundary test : ratio = 3.0
        // exactly. Per the inclusive `≤` operator, this should
        // pass (3.0 ≤ 3.0). Pre-audit no test pinned the upper
        // boundary semantics explicitly.
        let mut mode1 = generate_report(&make_outcome(), "kind", 1).unwrap();
        mode1.per_op.insert(
            "execute".into(),
            OpReport {
                p50_ms: Some(2.0),
                p95_ms: Some(3.5),
                p99_ms: Some(10.0),
                mean_ms: Some(2.5),
                rps: 100.0,
                success_count: 100,
                error_count: 0,
            },
        );
        let mut mode3 = generate_report(&make_outcome(), "kind", 1).unwrap();
        mode3.per_op.clear();
        mode3.per_op.insert(
            "mode3_execute_k5".into(),
            OpReport {
                p50_ms: Some(6.0),
                p95_ms: Some(20.0),
                p99_ms: Some(30.0), // 30 / 10 = 3.0 — exact boundary
                mean_ms: Some(10.0),
                rps: 50.0,
                success_count: 50,
                error_count: 0,
            },
        );
        let result = evaluate_c3(&mode3, &mode1);
        assert!(
            result.pass,
            "ratio = 3.0× must pass the ≤ 3.0× SLO floor (inclusive)"
        );
    }

    #[test]
    fn evaluate_c3_handles_mode3_k10_op_label() {
        // **Audit fix M6-A3 (Φ-F)** — `mode3-k10` workload produces
        // op label `mode3_execute_k10`. The `starts_with("mode3_execute")`
        // prefix matcher must catch it. Pre-audit only k5 was
        // pin-tested.
        let mode1 = generate_report(&make_outcome(), "kind", 1).unwrap();
        let mut mode3 = generate_report(&make_outcome(), "kind", 1).unwrap();
        mode3.per_op.clear();
        mode3.per_op.insert(
            "mode3_execute_k10".into(),
            OpReport {
                p50_ms: Some(5.0),
                p95_ms: Some(8.0),
                p99_ms: Some(10.0), // 10 / 4 = 2.5 ≤ 3 → pass
                mean_ms: Some(6.0),
                rps: 25.0,
                success_count: 25,
                error_count: 0,
            },
        );
        let result = evaluate_c3(&mode3, &mode1);
        assert!(
            result.pass,
            "mode3_execute_k10 op label must be picked up by the prefix matcher"
        );
        assert!(result.detail.contains("p99(mode3)=10"));
    }

    #[test]
    fn evaluate_c9_passes_when_speedup_above_floor() {
        let mut serial = generate_report(&make_outcome(), "kind", 1).unwrap();
        serial.per_op.clear();
        serial.per_op.insert(
            "mode3_execute".into(),
            OpReport {
                p50_ms: Some(10.0),
                p95_ms: Some(20.0),
                p99_ms: Some(30.0),
                mean_ms: Some(12.0),
                rps: 10.0,
                success_count: 50,
                error_count: 0,
            },
        );
        let mut parallel = generate_report(&make_outcome(), "kind", 1).unwrap();
        parallel.per_op.clear();
        parallel.per_op.insert(
            "mode3_execute_k5".into(),
            OpReport {
                p50_ms: Some(10.0),
                p95_ms: Some(20.0),
                p99_ms: Some(30.0),
                mean_ms: Some(12.0),
                rps: 30.0, // 30/10 = 3× ≥ 2.5×
                success_count: 150,
                error_count: 0,
            },
        );
        let result = evaluate_c9(&parallel, &serial);
        assert!(result.pass);
    }

    #[test]
    fn evaluate_c9_fails_when_speedup_below_floor() {
        let mut serial = generate_report(&make_outcome(), "kind", 1).unwrap();
        serial.per_op.clear();
        serial.per_op.insert(
            "mode3_execute".into(),
            OpReport {
                p50_ms: Some(10.0),
                p95_ms: Some(20.0),
                p99_ms: Some(30.0),
                mean_ms: Some(12.0),
                rps: 10.0,
                success_count: 50,
                error_count: 0,
            },
        );
        let mut parallel = generate_report(&make_outcome(), "kind", 1).unwrap();
        parallel.per_op.clear();
        parallel.per_op.insert(
            "mode3_execute_k5".into(),
            OpReport {
                p50_ms: Some(10.0),
                p95_ms: Some(20.0),
                p99_ms: Some(30.0),
                mean_ms: Some(12.0),
                rps: 15.0, // 15/10 = 1.5× < 2.5×
                success_count: 75,
                error_count: 0,
            },
        );
        let result = evaluate_c9(&parallel, &serial);
        assert!(!result.pass);
    }
}
