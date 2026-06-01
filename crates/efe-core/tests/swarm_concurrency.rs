//! **B5 — Concurrency tests for `efe_core::swarm::SwarmController`.**
//!
//! These verify the contract that `clone_handle()` produces concurrent-
//! safe controllers, and that high-concurrency `execute()` calls do not
//! deadlock, leak telemetry, or cross-pollinate state between requests.
//!
//! Coverage:
//! - **W4**: clone_handle is Send + Sync, multiple workers run in parallel
//! - 32 workers × 4 requests do not deadlock
//! - Each request's `Response.request_id` matches its `Request.id`
//! - Telemetry events from concurrent requests do not corrupt each other
//!   (each `begin:` matches one `finalize:`)
//! - A panic in one strategy invocation does NOT affect other in-flight
//!   requests (the runtime stays healthy)

mod mocks;

use std::sync::Arc;
use std::time::Duration;

use efe_core::fusion::FusionStats;
use efe_core::swarm::{
    ControllerConfig, Request, SwarmArbiter, SwarmController, Verdict,
};
use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};

use mocks::{
    MockAligner, MockDissolver, MockFusion, MockGenesis, RecordingTelemetry, ScriptedDetector,
};

fn spec(dim: u32, agents: u32, max_rounds: u32) -> SwarmSpec {
    SwarmSpec::builder()
        .problem_embedding(vec![0.0; dim as usize])
        .dim_latent(LatentDim::new(dim).expect("dim"))
        .population(PopulationMix::new(vec![(TemplateCode::NaSem, agents)]).expect("mix"))
        .max_rounds(max_rounds)
        .expect("rounds")
        .alignment_cone(
            AlignmentCone::new(vec![vec![0.1; dim as usize]], 0.15).expect("cone"),
        )
        .build()
        .expect("spec")
}

/// Build a happy-path controller. Returns the controller AND the
/// telemetry handle (for assertion).
fn build_controller(
    dim: u32,
    n_rounds: u32,
) -> (SwarmController, Arc<RecordingTelemetry>) {
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("dim"), 4));
    let stats: Vec<_> = (1..=n_rounds)
        .map(|r| FusionStats::synthetic_converging(r, dim as usize))
        .collect();
    let fusion = Arc::new(MockFusion::new(stats));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, "ok"));
    let aligner = Arc::new(MockAligner::safe());
    let telemetry = Arc::new(RecordingTelemetry::default());

    let last_round = n_rounds;
    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .telemetry(telemetry.clone())
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(move || {
            // Generate the verdicts inside the closure so each request
            // gets a fresh detector starting from round 1.
            let verdicts: Vec<_> = (1..last_round)
                .map(|_| Verdict::Continue)
                .chain(std::iter::once(Verdict::Converged { confidence: 0.9 }))
                .collect();
            Box::new(ScriptedDetector::new(verdicts))
        })
        .config(ControllerConfig::default())
        .build()
        .expect("build");

    (controller, telemetry)
}

// ====================================================================
// W4 — clone_handle works under concurrency
// ====================================================================

/// 32 workers × 4 requests via `clone_handle()`. Every Response must
/// match its Request by id, no request ids cross-pollinate.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_executes_via_clone_handle_no_deadlock_no_id_swap() {
    let dim = 4u32;
    let (controller, _telemetry) = build_controller(dim, 1);

    let workers = 32usize;
    let req_per_worker = 4usize;

    let mut joins = Vec::with_capacity(workers);
    for w in 0..workers {
        let ctrl = controller.clone_handle();
        joins.push(tokio::spawn(async move {
            let mut local_results = Vec::new();
            for r in 0..req_per_worker {
                let req = Request::for_tests(spec(dim, 4, 4));
                let req_id = req.id.clone();
                let label = format!("w{w}_r{r}");
                let resp = ctrl
                    .execute(req)
                    .await
                    .unwrap_or_else(|e| panic!("[{label}] execute failed: {e:?}"));
                assert_eq!(
                    resp.request_id, req_id,
                    "[{label}] response request_id != request id (id swap detected)"
                );
                local_results.push((label, resp));
            }
            local_results
        }));
    }

    let mut all_results = Vec::with_capacity(workers * req_per_worker);
    for join in joins {
        all_results.extend(join.await.expect("worker join"));
    }
    assert_eq!(all_results.len(), workers * req_per_worker);

    // Sanity: all swarm_ids must be distinct (UUID v4). We use a HashSet
    // since SwarmId implements Hash but not Ord.
    let swarm_ids: std::collections::HashSet<_> =
        all_results.iter().map(|(_, r)| r.swarm_id).collect();
    assert_eq!(
        swarm_ids.len(),
        workers * req_per_worker,
        "non-unique swarm_ids — uuid collision or request bleed"
    );
}

// ====================================================================
// Telemetry isolation — each begin: pairs with one finalize: in events
// ====================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_executes_telemetry_isolation() {
    let dim = 4u32;
    let (controller, telemetry) = build_controller(dim, 1);

    let workers = 16;
    let mut joins = Vec::with_capacity(workers);
    for _ in 0..workers {
        let ctrl = controller.clone_handle();
        joins.push(tokio::spawn(async move {
            let req = Request::for_tests(spec(dim, 4, 4));
            ctrl.execute(req).await.expect("execute");
        }));
    }
    for j in joins {
        j.await.expect("join");
    }

    let events = telemetry.events.lock().expect("lock");
    let begins = events.iter().filter(|e| e.starts_with("begin:")).count();
    let finalizes = events.iter().filter(|e| e.starts_with("finalize:")).count();
    assert_eq!(
        begins, workers,
        "expected {workers} begins, got {begins}; events={events:?}"
    );
    assert_eq!(
        finalizes, workers,
        "expected {workers} finalizes, got {finalizes}; events={events:?}"
    );
}

// ====================================================================
// No deadlock under sustained concurrent load
// ====================================================================

/// Run 64 requests through 8 worker tasks, each with a small jitter,
/// and ensure they all complete within a generous timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_executes_no_deadlock_under_jitter() {
    let dim = 4u32;
    let (controller, _telemetry) = build_controller(dim, 2);

    let workers = 8;
    let req_per_worker = 8;

    let result = tokio::time::timeout(
        Duration::from_secs(15),
        async {
            let mut joins = Vec::with_capacity(workers);
            for w in 0..workers {
                let ctrl = controller.clone_handle();
                joins.push(tokio::spawn(async move {
                    for r in 0..req_per_worker {
                        // jitter
                        tokio::time::sleep(Duration::from_micros(
                            ((w * 17 + r * 31) % 211) as u64,
                        ))
                        .await;
                        let req = Request::for_tests(spec(dim, 4, 4));
                        ctrl.execute(req).await.expect("execute");
                    }
                }));
            }
            for j in joins {
                j.await.expect("join");
            }
        },
    )
    .await;

    assert!(
        result.is_ok(),
        "deadlock or hang detected: 64 requests did not complete in 15s"
    );
}

// ====================================================================
// Panic isolation — one panicking request does not bring down the
// runtime or other in-flight requests
// ====================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn panic_in_one_request_does_not_kill_runtime() {
    use efe_core::errors::Result as EfeResult;
    use efe_core::swarm::FusionStrategy;

    /// Fusion that panics on the FIRST `run_round` call only.
    /// Subsequent calls return a normal converging stat.
    struct OneShotPanicFusion {
        used: std::sync::atomic::AtomicBool,
        stats: FusionStats,
    }
    #[async_trait::async_trait]
    impl FusionStrategy for OneShotPanicFusion {
        async fn run_round(
            &self,
            _: &mut efe_core::els::EpistemicLatentSpace,
            _: &efe_core::memory::NanoAgentBatch,
            _: u32,
        ) -> EfeResult<FusionStats> {
            if !self.used.swap(true, std::sync::atomic::Ordering::AcqRel) {
                panic!("intentional one-shot panic");
            }
            Ok(self.stats.clone())
        }
        async fn inject_noise(
            &self,
            _: &mut efe_core::els::EpistemicLatentSpace,
            _: f32,
        ) -> EfeResult<()> {
            Ok(())
        }
    }

    let dim = 4u32;
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("dim"), 4));
    let fusion = Arc::new(OneShotPanicFusion {
        used: std::sync::atomic::AtomicBool::new(false),
        stats: FusionStats::synthetic_converging(1, dim as usize),
    });
    let dissolver = Arc::new(MockDissolver::new(dim as usize, "ok"));
    let aligner = Arc::new(MockAligner::safe());
    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged {
                confidence: 0.9,
            }]))
        })
        .build()
        .expect("build");

    // Run two requests in parallel: first will hit the panic, second
    // (started slightly later via tokio::join!) will succeed.
    let ctrl1 = controller.clone_handle();
    let ctrl2 = controller.clone_handle();

    let task1 = tokio::spawn(async move {
        let req = Request::for_tests(spec(dim, 4, 4));
        ctrl1.execute(req).await
    });
    // Small delay so task1's first run_round panic happens BEFORE task2's.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let task2 = tokio::spawn(async move {
        let req = Request::for_tests(spec(dim, 4, 4));
        ctrl2.execute(req).await
    });

    let r1 = task1.await;
    let r2 = task2.await;

    // task1 must end as a JoinError (panic) OR an EfeError (the panic
    // unwinds through the executor and the controller propagates an
    // appropriate error). In tokio, an unwinding panic becomes a
    // JoinError::is_panic() — the spawn returns Err.
    assert!(
        r1.is_err() || r1.as_ref().is_ok_and(|inner| inner.is_err()),
        "task1 should have failed"
    );

    // task2 must succeed: the runtime is still alive after task1's panic.
    let resp = r2
        .expect("task2 join")
        .expect("task2 execute should succeed");
    assert_eq!(resp.rounds_executed, 1);
}
