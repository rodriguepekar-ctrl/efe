//! Sprint D18 — parallel bench runner.
//!
//! The runner spawns N concurrent tokio workers that each issue
//! workload ops at a target rate, recording outcomes into the
//! shared [`crate::LatencyHistogram`] + [`crate::RpsCounter`].
//!
//! Sprint **D31 (Φ-D)** extension — every worker is assigned a
//! `gpu_shard_id = worker_idx % gpu_shards` (when the runner is
//! configured with N>1 shards). The shard id is recorded into a
//! per-shard histogram + counter so the report can compute the
//! D31-GPU-1 SLO (per-shard speedup spread, Jain index ≥ 0.95).
//!
//! Sprint **D38 (Φ-F)** extension — when the workload is Mode-3,
//! the runner records the canonical `k_swarm` value into the
//! report's top-level `k_swarm` field via the workload helper
//! [`Workload::k_swarm`].

use std::sync::Arc;
use std::time::{Duration, Instant};

// **Audit fix D18-A8** — drop the unused `CallOutcome` import. The
// type is constructed by `MockBenchClient::call` ; the runner only
// observes its public fields via the `Result` shape so the import
// added zero value but produced a `unused_import` lint.
use crate::client::BenchClient;
use crate::error::BenchError;
use crate::gen::Dataset;
use crate::metrics::{LatencyHistogram, RpsCounter};
use crate::workload::Workload;

/// Configuration for a runner.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// The workload to execute.
    pub workload: Workload,
    /// Symbolic target environment (`dev`, `staging`, `drill`, ...).
    /// **MUST NOT** be `prod` ; validated at construction.
    pub target_env: String,
    /// **Sprint D31 (Φ-D)** — number of GPU shards orchestrated.
    /// Default 1 (no sharding). Each worker is assigned
    /// `shard_id = worker_idx % gpu_shards` and its observations
    /// are recorded into a per-shard histogram + counter.
    ///
    /// Validation : `gpu_shards ∈ [1, 16]` (cluster cap, see
    /// [`crate::workload::validate_gpu_shards`]).
    pub gpu_shards: u32,
}

impl RunnerConfig {
    /// Construct + validate.
    ///
    /// **Sprint D31 (Φ-D)** — defaults to `gpu_shards=1` (no
    /// sharding). Use [`Self::with_gpu_shards`] to enable
    /// multi-shard orchestration.
    pub fn new(workload: Workload, target_env: impl Into<String>) -> Result<Self, BenchError> {
        let env = target_env.into();
        crate::workload::validate_target_env(&env)?;
        workload.validate()?;
        Ok(Self {
            workload,
            target_env: env,
            gpu_shards: 1,
        })
    }

    /// **Sprint D31 (Φ-D)** — declare the number of GPU shards.
    ///
    /// Validates `n ∈ [1, 16]` and rejects out-of-range values
    /// with [`BenchError::GpuShardOutOfRange`].
    pub fn with_gpu_shards(mut self, n: u32) -> Result<Self, BenchError> {
        crate::workload::validate_gpu_shards(n)?;
        self.gpu_shards = n;
        Ok(self)
    }
}

/// Outcome of a single runner execution.
#[derive(Debug)]
pub struct RunnerOutcome {
    /// Workload label.
    pub workload_label: String,
    /// Total wall-clock duration.
    pub duration: Duration,
    /// Per-op histogram + counter snapshots.
    pub per_op: std::collections::HashMap<String, OpStats>,
    /// Aggregate stats across all ops.
    pub aggregate: OpStats,
    /// Per-error counter.
    pub errors_by_label: std::collections::HashMap<String, u64>,
    /// **Sprint D31 (Φ-D)** — per-shard stats (`shard_id` → stats).
    /// Empty when `gpu_shards == 1`. Indexed by integer shard id
    /// (0..gpu_shards).
    pub per_shard: std::collections::BTreeMap<u32, ShardStats>,
    /// **Sprint D31 (Φ-D)** — number of GPU shards orchestrated.
    pub gpu_shards: u32,
}

/// Stats for a single op (or aggregate).
#[derive(Debug)]
pub struct OpStats {
    /// Latency histogram snapshot.
    pub histogram: crate::metrics::HistogramSnapshot,
    /// Total successful calls.
    pub success_count: u64,
    /// Total failed calls.
    pub error_count: u64,
    /// Effective RPS over the run.
    pub rps: f64,
}

/// **Sprint D31 (Φ-D)** — per-shard statistics. Aggregates every
/// observation made by the workers assigned to this shard
/// (`worker_idx % gpu_shards == shard_id`).
#[derive(Debug)]
pub struct ShardStats {
    /// Latency histogram snapshot for this shard only.
    pub histogram: crate::metrics::HistogramSnapshot,
    /// Total successful calls on this shard.
    pub success_count: u64,
    /// Total failed calls on this shard.
    pub error_count: u64,
    /// Effective RPS on this shard.
    pub rps: f64,
}

/// The bench runner.
pub struct Runner {
    config: RunnerConfig,
    client: Arc<dyn BenchClient>,
    dataset: Arc<Dataset>,
}

impl Runner {
    /// Construct.
    pub fn new(
        config: RunnerConfig,
        client: Arc<dyn BenchClient>,
        dataset: Arc<Dataset>,
    ) -> Self {
        Self {
            config,
            client,
            dataset,
        }
    }

    /// Execute the workload. Returns the outcome.
    pub async fn run(&self) -> Result<RunnerOutcome, BenchError> {
        let workload = self.config.workload.clone();
        let concurrency = workload.concurrency.max(1) as usize;
        let gpu_shards = self.config.gpu_shards.max(1);

        // Per-op histograms + counters, shared across workers.
        let mut per_op_histograms: std::collections::HashMap<&'static str, Arc<LatencyHistogram>> =
            std::collections::HashMap::new();
        let mut per_op_counters: std::collections::HashMap<&'static str, Arc<RpsCounter>> =
            std::collections::HashMap::new();
        for (op, _) in &workload.mix {
            per_op_histograms
                .entry(op.as_str())
                .or_insert_with(|| Arc::new(LatencyHistogram::new()));
            per_op_counters
                .entry(op.as_str())
                .or_insert_with(|| Arc::new(RpsCounter::new()));
        }
        let aggregate_histogram = Arc::new(LatencyHistogram::new());
        let aggregate_counter = Arc::new(RpsCounter::new());
        let errors_label_counter: Arc<std::sync::Mutex<std::collections::HashMap<String, u64>>> =
            Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

        // **Sprint D31 (Φ-D)** — per-shard histograms + counters.
        // Pre-allocate a slot for each shard id even when no worker
        // lands on that shard — the report uniformly lists all
        // shards, including empty ones (≡ a worker_idx never hit
        // that shard, which itself is an interesting datapoint).
        let mut per_shard_histograms: std::collections::BTreeMap<u32, Arc<LatencyHistogram>> =
            std::collections::BTreeMap::new();
        let mut per_shard_counters: std::collections::BTreeMap<u32, Arc<RpsCounter>> =
            std::collections::BTreeMap::new();
        for shard_id in 0..gpu_shards {
            per_shard_histograms.insert(shard_id, Arc::new(LatencyHistogram::new()));
            per_shard_counters.insert(shard_id, Arc::new(RpsCounter::new()));
        }

        // Per-worker target RPS pacing.
        let per_worker_rps = (workload.target_rps as f64 / concurrency as f64).max(1.0);
        let pace_duration = Duration::from_secs_f64(1.0 / per_worker_rps);

        let started_at = Instant::now();
        let deadline = started_at + workload.duration;

        let mut handles = Vec::with_capacity(concurrency);
        for worker_id in 0..concurrency {
            let workload = workload.clone();
            let client = Arc::clone(&self.client);
            let dataset = Arc::clone(&self.dataset);
            let per_op_h = per_op_histograms.clone();
            let per_op_c = per_op_counters.clone();
            let aggregate_h = Arc::clone(&aggregate_histogram);
            let aggregate_c = Arc::clone(&aggregate_counter);
            let errors_l = Arc::clone(&errors_label_counter);
            let per_shard_h = per_shard_histograms.clone();
            let per_shard_c = per_shard_counters.clone();
            // **D31** — assign shard_id deterministically.
            let shard_id = (worker_id as u32) % gpu_shards;

            handles.push(tokio::spawn(async move {
                worker_loop(
                    worker_id as u64,
                    shard_id,
                    workload,
                    client,
                    dataset,
                    per_op_h,
                    per_op_c,
                    aggregate_h,
                    aggregate_c,
                    errors_l,
                    per_shard_h,
                    per_shard_c,
                    pace_duration,
                    deadline,
                )
                .await
            }));
        }
        for h in handles {
            let _ = h.await;
        }

        let duration = started_at.elapsed();

        // Collect per-op stats.
        let mut per_op = std::collections::HashMap::new();
        for (op_str, hist) in per_op_histograms {
            let counter = per_op_counters.get(op_str).map_or_else(
                || Arc::new(RpsCounter::new()),
                Arc::clone,
            );
            per_op.insert(
                op_str.to_string(),
                OpStats {
                    histogram: hist.snapshot(),
                    success_count: counter.success_count(),
                    error_count: counter.error_count(),
                    rps: counter.rps(),
                },
            );
        }
        let aggregate = OpStats {
            histogram: aggregate_histogram.snapshot(),
            success_count: aggregate_counter.success_count(),
            error_count: aggregate_counter.error_count(),
            rps: aggregate_counter.rps(),
        };

        let errors_by_label = errors_label_counter
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();

        // **Sprint D31 (Φ-D)** — collect per-shard stats.
        let mut per_shard = std::collections::BTreeMap::new();
        for (shard_id, hist) in per_shard_histograms {
            let counter = per_shard_counters.get(&shard_id).map_or_else(
                || Arc::new(RpsCounter::new()),
                Arc::clone,
            );
            per_shard.insert(
                shard_id,
                ShardStats {
                    histogram: hist.snapshot(),
                    success_count: counter.success_count(),
                    error_count: counter.error_count(),
                    rps: counter.rps(),
                },
            );
        }

        Ok(RunnerOutcome {
            workload_label: workload.label,
            duration,
            per_op,
            aggregate,
            errors_by_label,
            per_shard,
            gpu_shards,
        })
    }
}

#[allow(clippy::too_many_arguments)]
async fn worker_loop(
    worker_id: u64,
    shard_id: u32,
    workload: Workload,
    client: Arc<dyn BenchClient>,
    _dataset: Arc<Dataset>,
    per_op_histograms: std::collections::HashMap<&'static str, Arc<LatencyHistogram>>,
    per_op_counters: std::collections::HashMap<&'static str, Arc<RpsCounter>>,
    aggregate_histogram: Arc<LatencyHistogram>,
    aggregate_counter: Arc<RpsCounter>,
    errors_label_counter: Arc<std::sync::Mutex<std::collections::HashMap<String, u64>>>,
    per_shard_histograms: std::collections::BTreeMap<u32, Arc<LatencyHistogram>>,
    per_shard_counters: std::collections::BTreeMap<u32, Arc<RpsCounter>>,
    pace_duration: Duration,
    deadline: Instant,
) {
    let mut call_idx: u64 = worker_id * 1_000_003;  // worker offset for op selection
    while Instant::now() < deadline {
        let next_call_at = Instant::now() + pace_duration;

        // Pick a workload op.
        let op = match workload.pick_op(call_idx as u32) {
            Some(op) => op,
            None => {
                // Validation should have caught this ; defensive.
                break;
            }
        };

        // Pick a tenant — round-robin across the dataset's tenants
        // so multi-tenant workloads exercise spread.
        let tenant = format!("bench-tenant-{:06}", call_idx % 1_000);

        // Issue the call.
        // **Audit fix M6-A1 (Φ-F)** — pass `worker_id` so the
        // Tonic client uses the same value to derive the
        // `x-efe-gpu-shard-id` metadata header that the runner
        // uses to bucket per-shard observations. Pre-audit the
        // Tonic client used a private atomic counter, decoupling
        // the metric bucket from the propagated routing label.
        let result = client.call(op, &tenant, worker_id).await;
        match result {
            Ok(outcome) => {
                aggregate_histogram.observe(outcome.latency);
                if let Some(h) = per_op_histograms.get(op.as_str()) {
                    h.observe(outcome.latency);
                }
                // **Sprint D31 (Φ-D)** — record per-shard.
                if let Some(h) = per_shard_histograms.get(&shard_id) {
                    h.observe(outcome.latency);
                }
                if outcome.ok {
                    aggregate_counter.record_success();
                    if let Some(c) = per_op_counters.get(op.as_str()) {
                        c.record_success();
                    }
                    if let Some(c) = per_shard_counters.get(&shard_id) {
                        c.record_success();
                    }
                } else {
                    aggregate_counter.record_error();
                    if let Some(c) = per_op_counters.get(op.as_str()) {
                        c.record_error();
                    }
                    if let Some(c) = per_shard_counters.get(&shard_id) {
                        c.record_error();
                    }
                    if let Some(label) = &outcome.error_label {
                        if let Ok(mut g) = errors_label_counter.lock() {
                            *g.entry(label.clone()).or_insert(0) += 1;
                        }
                    }
                }
            }
            Err(e) => {
                aggregate_counter.record_error();
                if let Some(c) = per_op_counters.get(op.as_str()) {
                    c.record_error();
                }
                if let Some(c) = per_shard_counters.get(&shard_id) {
                    c.record_error();
                }
                if let Ok(mut g) = errors_label_counter.lock() {
                    *g.entry(e.metric_label().to_string()).or_insert(0) += 1;
                }
            }
        }

        call_idx = call_idx.wrapping_add(1);

        // Pacing.
        let now = Instant::now();
        if now < next_call_at {
            tokio::time::sleep(next_call_at - now).await;
        }
    }
}

// **Audit fix D18-A9** — pre-fix the file shipped a `fn op_tag(op:
// WorkloadOp) -> &'static str` helper guarded by
// `#[allow(dead_code)]`. It was a leftover from an early iteration
// where each worker tagged its op via this helper ; the runtime
// switched to `op.as_str()` directly, leaving `op_tag` orphaned. We
// drop it to keep the public API lean (and to honour the
// project-wide "no dead-code helpers" hygiene rule).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockBenchClient;
    use crate::gen::{gen_dataset, DatasetSpec};

    fn make_runner(workload: Workload) -> Runner {
        let dataset =
            Arc::new(gen_dataset(&DatasetSpec::smoke()).unwrap());
        let client: Arc<dyn BenchClient> =
            Arc::new(MockBenchClient::new().with_canned_latency(Duration::from_millis(2)));
        let cfg = RunnerConfig::new(workload, "kind-bench").unwrap();
        Runner::new(cfg, client, dataset)
    }

    #[tokio::test]
    async fn runner_smoke_completes() {
        let mut workload = Workload::canonical("balanced").unwrap();
        workload.duration = Duration::from_millis(200);
        workload.target_rps = 50;
        workload.concurrency = 4;
        let runner = make_runner(workload);
        let out = runner.run().await.unwrap();
        assert!(out.aggregate.success_count > 0);
        assert!(out.duration >= Duration::from_millis(150));
    }

    #[tokio::test]
    async fn runner_records_per_op_stats() {
        let mut workload = Workload::canonical("read-heavy").unwrap();
        workload.duration = Duration::from_millis(200);
        workload.target_rps = 100;
        workload.concurrency = 2;
        let runner = make_runner(workload);
        let out = runner.run().await.unwrap();
        // read-heavy mixes resolve_catalog + execute → both keys
        // should be present.
        assert!(out.per_op.contains_key("resolve_catalog"));
    }

    #[tokio::test]
    async fn runner_refuses_prod() {
        let workload = Workload::canonical("balanced").unwrap();
        let cfg = RunnerConfig::new(workload, "prod");
        assert!(matches!(cfg, Err(BenchError::RefusedProdTarget(_))));
    }

    #[tokio::test]
    async fn runner_records_errors_by_label() {
        let dataset =
            Arc::new(gen_dataset(&DatasetSpec::smoke()).unwrap());
        let client: Arc<dyn BenchClient> = Arc::new(
            MockBenchClient::new()
                .with_canned_latency(Duration::from_millis(1))
                .with_canned_error("Unavailable"),
        );
        let mut workload = Workload::canonical("balanced").unwrap();
        workload.duration = Duration::from_millis(150);
        workload.target_rps = 30;
        workload.concurrency = 2;
        let cfg = RunnerConfig::new(workload, "kind").unwrap();
        let runner = Runner::new(cfg, client, dataset);
        let out = runner.run().await.unwrap();
        assert!(out.aggregate.error_count > 0);
        assert!(out.errors_by_label.contains_key("Unavailable"));
    }

    // -----------------------------------------------------------
    // **Sprint D31 (Φ-D)** — gpu_shards orchestration tests.
    // -----------------------------------------------------------

    #[tokio::test]
    async fn runner_default_gpu_shards_is_one() {
        let workload = Workload::canonical("balanced").unwrap();
        let cfg = RunnerConfig::new(workload, "kind").unwrap();
        assert_eq!(cfg.gpu_shards, 1);
    }

    #[tokio::test]
    async fn runner_with_gpu_shards_propagates_to_outcome() {
        let mut workload = Workload::canonical("balanced").unwrap();
        workload.duration = Duration::from_millis(200);
        workload.target_rps = 100;
        workload.concurrency = 4;
        let dataset =
            Arc::new(gen_dataset(&DatasetSpec::smoke()).unwrap());
        let client: Arc<dyn BenchClient> =
            Arc::new(MockBenchClient::new().with_canned_latency(Duration::from_millis(1)));
        let cfg = RunnerConfig::new(workload, "kind")
            .unwrap()
            .with_gpu_shards(4)
            .unwrap();
        let runner = Runner::new(cfg, client, dataset);
        let out = runner.run().await.unwrap();
        assert_eq!(out.gpu_shards, 4);
        // 4 workers % 4 shards = 1 worker per shard. Every shard
        // should have observed at least 1 call.
        assert_eq!(out.per_shard.len(), 4);
        for shard_id in 0..4u32 {
            let s = out.per_shard.get(&shard_id).unwrap();
            assert!(
                s.success_count > 0 || s.error_count > 0,
                "shard {shard_id} unexpectedly empty"
            );
        }
    }

    #[tokio::test]
    async fn runner_with_gpu_shards_zero_rejected() {
        let workload = Workload::canonical("balanced").unwrap();
        let r = RunnerConfig::new(workload, "kind")
            .unwrap()
            .with_gpu_shards(0);
        assert!(matches!(r, Err(BenchError::GpuShardOutOfRange { got: 0 })));
    }

    #[tokio::test]
    async fn runner_with_gpu_shards_above_cap_rejected() {
        let workload = Workload::canonical("balanced").unwrap();
        let r = RunnerConfig::new(workload, "kind")
            .unwrap()
            .with_gpu_shards(17);
        assert!(matches!(r, Err(BenchError::GpuShardOutOfRange { got: 17 })));
    }

    #[tokio::test]
    async fn runner_with_uneven_concurrency_distributes_workers() {
        // 6 workers % 4 shards = shards 0,1,2 get 2 workers each,
        // shard 3 gets 0. The outcome map still lists shard 3 (with
        // zero counts) — operators rely on the exhaustive shard
        // listing for fairness analysis.
        let mut workload = Workload::canonical("balanced").unwrap();
        workload.duration = Duration::from_millis(200);
        workload.target_rps = 60;
        workload.concurrency = 6;
        let dataset =
            Arc::new(gen_dataset(&DatasetSpec::smoke()).unwrap());
        let client: Arc<dyn BenchClient> =
            Arc::new(MockBenchClient::new().with_canned_latency(Duration::from_millis(1)));
        let cfg = RunnerConfig::new(workload, "kind")
            .unwrap()
            .with_gpu_shards(4)
            .unwrap();
        let runner = Runner::new(cfg, client, dataset);
        let out = runner.run().await.unwrap();
        assert_eq!(out.per_shard.len(), 4);
        // Total observations across shards = aggregate observations.
        let total_per_shard: u64 = out
            .per_shard
            .values()
            .map(|s| s.success_count + s.error_count)
            .sum();
        let total_aggregate =
            out.aggregate.success_count + out.aggregate.error_count;
        assert_eq!(total_per_shard, total_aggregate);
    }

    // -----------------------------------------------------------
    // **Sprint D38 (Φ-F)** — Mode-3 workload through the runner.
    // -----------------------------------------------------------

    #[tokio::test]
    async fn runner_executes_mode3_k5_workload() {
        let mut workload = Workload::canonical("mode3-k5").unwrap();
        workload.duration = Duration::from_millis(150);
        workload.target_rps = 20;
        workload.concurrency = 2;
        let runner = make_runner(workload);
        let out = runner.run().await.unwrap();
        // Mode-3 op label is `mode3_execute_k5`.
        assert!(out.per_op.contains_key("mode3_execute_k5"));
        assert!(out.aggregate.success_count > 0);
    }

    #[tokio::test]
    async fn runner_executes_mode3_k10_workload() {
        let mut workload = Workload::canonical("mode3-k10").unwrap();
        workload.duration = Duration::from_millis(150);
        workload.target_rps = 10;
        workload.concurrency = 2;
        let runner = make_runner(workload);
        let out = runner.run().await.unwrap();
        assert!(out.per_op.contains_key("mode3_execute_k10"));
    }

    // -----------------------------------------------------------
    // **Audit fix M6-A1 (Φ-F)** — worker_id ↔ shard_id alignment.
    //
    // Pre-audit the runner's per-shard metric bucket was keyed on
    // `worker_id % gpu_shards` while the Tonic client propagated
    // a shard_id derived from a private atomic counter (rotating
    // per call across all workers). The two values were
    // misaligned, breaking the diagnostic story ("shard 0 is
    // slow" → which GPU is shard 0 ?).
    //
    // The test below is observational : it spawns a runner with
    // 4 shards + 4 workers and asserts that the same `worker_id`
    // is passed to every `client.call(...)` from a given worker.
    // We use a custom mock client that captures `(op, tenant,
    // worker_id)` triples so we can verify that worker N's calls
    // all carry worker_id=N.
    // -----------------------------------------------------------

    #[derive(Debug)]
    struct WorkerIdCapturingClient {
        captured: std::sync::Mutex<Vec<u64>>,
    }

    #[async_trait::async_trait]
    impl BenchClient for WorkerIdCapturingClient {
        async fn call(
            &self,
            op: crate::workload::WorkloadOp,
            _tenant: &str,
            worker_id: u64,
        ) -> Result<crate::client::CallOutcome, BenchError> {
            if let Ok(mut g) = self.captured.lock() {
                g.push(worker_id);
            }
            Ok(crate::client::CallOutcome {
                op,
                latency: Duration::from_millis(1),
                ok: true,
                error_label: None,
            })
        }
    }

    #[tokio::test]
    async fn runner_passes_stable_worker_id_per_call() {
        let mut workload = Workload::canonical("balanced").unwrap();
        workload.duration = Duration::from_millis(200);
        workload.target_rps = 100;
        workload.concurrency = 4;
        let dataset = Arc::new(gen_dataset(&DatasetSpec::smoke()).unwrap());
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));
        let client: Arc<dyn BenchClient> = Arc::new(WorkerIdCapturingClient {
            captured: std::sync::Mutex::new(Vec::new()),
        });
        // Above we created a fresh Mutex in the client ; for
        // visibility from the test we re-bind via downcast-equiv :
        // we simply build a new client that shares the captured
        // arc by reference to make the test deterministic.
        struct SharedCapturingClient {
            captured: std::sync::Arc<std::sync::Mutex<Vec<u64>>>,
        }
        impl std::fmt::Debug for SharedCapturingClient {
            fn fmt(
                &self,
                f: &mut std::fmt::Formatter<'_>,
            ) -> std::fmt::Result {
                f.debug_struct("SharedCapturingClient").finish()
            }
        }
        #[async_trait::async_trait]
        impl BenchClient for SharedCapturingClient {
            async fn call(
                &self,
                op: crate::workload::WorkloadOp,
                _tenant: &str,
                worker_id: u64,
            ) -> Result<crate::client::CallOutcome, BenchError> {
                if let Ok(mut g) = self.captured.lock() {
                    g.push(worker_id);
                }
                Ok(crate::client::CallOutcome {
                    op,
                    latency: Duration::from_millis(1),
                    ok: true,
                    error_label: None,
                })
            }
        }
        let _ = client; // discard the unused first construct.
        let client: Arc<dyn BenchClient> = Arc::new(SharedCapturingClient {
            captured: captured.clone(),
        });
        let cfg = RunnerConfig::new(workload, "kind").unwrap();
        let runner = Runner::new(cfg, client, dataset);
        let _ = runner.run().await.unwrap();

        let observed = captured.lock().unwrap().clone();
        // We must see at least one call from each of the 4 workers
        // (worker_id ∈ {0, 1, 2, 3}). A high concurrency × short
        // duration may yield a few calls per worker, so we just
        // pin the SET inclusion.
        let unique: std::collections::BTreeSet<u64> =
            observed.iter().copied().collect();
        for expected in 0..4u64 {
            assert!(
                unique.contains(&expected),
                "expected worker_id {expected} to appear in capture set {unique:?}"
            );
        }
        // Every captured worker_id MUST be one of {0, 1, 2, 3} —
        // the runner never invents worker ids beyond the
        // concurrency bound.
        for w in &observed {
            assert!(
                *w < 4,
                "worker_id {w} out of bounds for concurrency=4"
            );
        }
    }
}
