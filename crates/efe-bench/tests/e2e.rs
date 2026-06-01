//! Sprint **D18** — `efe-bench` integration tests.
//!
//! End-to-end tests of the bench harness using the
//! [`efe_bench::MockBenchClient`]. No real cluster needed.

use std::sync::Arc;
use std::time::Duration;

use efe_bench::{
    client::{BenchClient, MockBenchClient},
    error::BenchError,
    gen::{gen_dataset, DatasetSpec},
    report::{generate_report, render_markdown},
    runner::{Runner, RunnerConfig},
    workload::{Workload, WorkloadOp},
};

#[tokio::test]
async fn end_to_end_balanced_workload_completes() {
    let mut workload = Workload::canonical("balanced").unwrap();
    workload.duration = Duration::from_millis(300);
    workload.target_rps = 50;
    workload.concurrency = 4;
    let cfg = RunnerConfig::new(workload, "kind-bench").unwrap();

    let dataset = Arc::new(gen_dataset(&DatasetSpec::smoke()).unwrap());
    let client: Arc<dyn BenchClient> = Arc::new(
        MockBenchClient::new().with_canned_latency(Duration::from_millis(3)),
    );
    let runner = Runner::new(cfg, client, dataset);

    let outcome = runner.run().await.unwrap();
    assert!(outcome.aggregate.success_count > 0);
    assert!(outcome.duration >= Duration::from_millis(250));
}

#[tokio::test]
async fn end_to_end_report_generation_and_markdown_render() {
    let mut workload = Workload::canonical("read-heavy").unwrap();
    workload.duration = Duration::from_millis(200);
    workload.target_rps = 30;
    workload.concurrency = 2;
    let cfg = RunnerConfig::new(workload, "kind").unwrap();
    let dataset = Arc::new(gen_dataset(&DatasetSpec::smoke()).unwrap());
    let client: Arc<dyn BenchClient> = Arc::new(MockBenchClient::new());
    let runner = Runner::new(cfg, client, dataset);

    let outcome = runner.run().await.unwrap();
    let report = generate_report(&outcome, "kind", 3).unwrap();

    // Sanity on report shape.
    assert_eq!(report.workload, "read-heavy");
    assert_eq!(report.replicas, 3);
    assert!(!report.slos.is_empty());

    // Render markdown — non-empty.
    let md = render_markdown(&report);
    // **Sprint D29 (Φ-D)** — markdown header bumped from
    // "# Sprint D18 — Bench Results" (D18 era) to the more
    // durable "# EFE Bench Report" since post-D29 the harness
    // covers way more than the D18 sprint scope (real Tonic
    // client, multi-GPU sharding, Mode-3 workloads). The 7
    // D18-bench-N SLOs remain in the SLO results section so
    // existing dashboards keying on those identifiers still
    // render correctly.
    assert!(md.contains("# EFE Bench Report"));
    assert!(md.contains("D18-bench-"));
}

#[tokio::test]
async fn end_to_end_refuses_prod_target() {
    let workload = Workload::canonical("balanced").unwrap();
    let cfg = RunnerConfig::new(workload, "prod");
    assert!(matches!(cfg, Err(BenchError::RefusedProdTarget(_))));
}

#[tokio::test]
async fn end_to_end_records_errors_in_report() {
    let mut workload = Workload::canonical("balanced").unwrap();
    workload.duration = Duration::from_millis(150);
    workload.target_rps = 30;
    workload.concurrency = 2;
    let cfg = RunnerConfig::new(workload, "kind").unwrap();
    let dataset = Arc::new(gen_dataset(&DatasetSpec::smoke()).unwrap());
    let client: Arc<dyn BenchClient> = Arc::new(
        MockBenchClient::new()
            .with_canned_latency(Duration::from_millis(2))
            .with_canned_error("Unavailable"),
    );
    let runner = Runner::new(cfg, client, dataset);

    let outcome = runner.run().await.unwrap();
    let report = generate_report(&outcome, "kind", 1).unwrap();

    assert!(report.error_count > 0);
    assert!(report.errors_by_label.contains_key("Unavailable"));
    // No SLO involves error rates explicitly in v0.1.19, but
    // the report carries the breakdown.
}

#[tokio::test]
async fn end_to_end_per_op_breakdown_present() {
    // The read-heavy workload mixes ResolveCatalog + Execute.
    let mut workload = Workload::canonical("read-heavy").unwrap();
    workload.duration = Duration::from_millis(200);
    workload.target_rps = 100;
    workload.concurrency = 4;
    let cfg = RunnerConfig::new(workload, "kind").unwrap();
    let dataset = Arc::new(gen_dataset(&DatasetSpec::smoke()).unwrap());
    let client: Arc<dyn BenchClient> = Arc::new(MockBenchClient::new());
    let runner = Runner::new(cfg, client, dataset);
    let outcome = runner.run().await.unwrap();
    let report = generate_report(&outcome, "kind", 1).unwrap();

    assert!(report.per_op.contains_key("resolve_catalog"));
}

#[tokio::test]
async fn end_to_end_dataset_deterministic() {
    let a = gen_dataset(&DatasetSpec::tier1_typical()).unwrap();
    let b = gen_dataset(&DatasetSpec::tier1_typical()).unwrap();
    // Compare the first sentence — sufficient for a determinism check.
    let a_s = &a.tenants[0].catalogs[0].classes[0].sentences[0];
    let b_s = &b.tenants[0].catalogs[0].classes[0].sentences[0];
    assert_eq!(a_s, b_s);
}

#[tokio::test]
async fn end_to_end_workload_op_pick_distribution() {
    // 100 % Execute → every pick must be Execute.
    let workload = Workload {
        label: "all-execute".into(),
        mix: vec![(WorkloadOp::Execute, 1)],
        target_rps: 1,
        duration: Duration::from_millis(1),
        concurrency: 1,
    };
    workload.validate().unwrap();
    for i in 0..100 {
        assert_eq!(workload.pick_op(i).unwrap(), WorkloadOp::Execute);
    }
}
