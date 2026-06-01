//! End-to-end scenarios for [`SwarmController::execute`] using the mocks
//! in `mocks.rs`.
//!
//! These cover the happy path, convergence at various rounds, divergence,
//! mode-collapse recovery, budget exhaustion, and alignment breach.

mod mocks;

use std::sync::Arc;

use efe_core::fusion::FusionStats;
use efe_core::swarm::{
    ControllerConfig, DecodeMode, DecodedOutput, Request, SwarmArbiter, SwarmController, Verdict,
};
use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};
use efe_core::EfeError;

use mocks::{
    FailingDissolver, MockAligner, MockDissolver, MockFusion, MockGenesis, RecordingTelemetry,
    ScriptedDetector,
};

// ---------- Helpers ----------

fn spec(dim: u32, agents: u32, max_rounds: u32) -> SwarmSpec {
    SwarmSpec::builder()
        .problem_embedding(vec![0.0; dim as usize])
        .dim_latent(LatentDim::new(dim).expect("valid dim"))
        .population(PopulationMix::new(vec![(TemplateCode::NaSem, agents)]).expect("valid mix"))
        .max_rounds(max_rounds)
        .expect("valid rounds")
        .alignment_cone(
            AlignmentCone::new(vec![vec![0.1; dim as usize]], 0.15).expect("valid cone"),
        )
        .build()
        .expect("valid spec")
}

fn convergence_stats(rounds: u32, dim: usize) -> Vec<FusionStats> {
    (1..=rounds)
        .map(|r| FusionStats::synthetic_converging(r, dim))
        .collect()
}

// ---------- Happy path ----------

#[tokio::test]
async fn controller_converges_on_third_round() {
    let dim = 4;
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("valid"), 10));
    let fusion = Arc::new(MockFusion::new(convergence_stats(3, dim as usize)));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, "answer"));
    let aligner = Arc::new(MockAligner::safe());
    let telemetry = Arc::new(RecordingTelemetry::default());

    let verdicts = vec![
        Verdict::Continue,
        Verdict::Continue,
        Verdict::Converged { confidence: 0.93 },
    ];

    let controller = SwarmController::builder()
        .genesis(genesis.clone())
        .fusion(fusion.clone())
        .dissolver(dissolver)
        .alignment(aligner.clone())
        .telemetry(telemetry.clone())
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(move || Box::new(ScriptedDetector::new(verdicts.clone())))
        .config(ControllerConfig::default())
        .build()
        .expect("build controller");

    let req = Request::for_tests(spec(dim, 10, 8));
    let resp = controller.execute(req).await.expect("success");

    assert_eq!(resp.rounds_executed, 3);
    assert!((resp.confidence - 0.93).abs() < 1e-6);
    assert_eq!(*genesis.spawn_calls.lock().expect("lock"), 1);
    assert_eq!(*fusion.round_calls.lock().expect("lock"), 3);
    assert_eq!(*aligner.calls.lock().expect("lock"), 1);

    let events = telemetry.events.lock().expect("lock");
    assert!(events.iter().any(|e| e.starts_with("begin:")));
    assert!(events
        .iter()
        .any(|e| e.starts_with("phase_complete:genesis")));
    assert!(events.iter().any(|e| e.starts_with("round:3")));
    assert!(events.iter().any(|e| e.starts_with("finalize:")));
}

// ---------- Text vs pass-through decoding ----------

#[tokio::test]
async fn controller_passthrough_returns_raw_centroid() {
    let dim = 8;
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("valid"), 16));
    let fusion = Arc::new(MockFusion::new(convergence_stats(1, dim as usize)));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, "n/a"));
    let aligner = Arc::new(MockAligner::safe());

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged {
                confidence: 0.9,
            }]))
        })
        .build()
        .expect("build");

    let mut req = Request::for_tests(spec(dim, 16, 4));
    req.decode_mode = DecodeMode::PassThrough;

    let resp = controller.execute(req).await.expect("success");
    assert!(matches!(resp.output, DecodedOutput::Raw(_)));
}

// ---------- Divergence ----------

#[tokio::test]
async fn controller_propagates_divergence() {
    let dim = 4;
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("valid"), 8));
    let fusion = Arc::new(MockFusion::new(convergence_stats(2, dim as usize)));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, ""));
    let aligner = Arc::new(MockAligner::safe());

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![
                Verdict::Continue,
                Verdict::Divergence {
                    reason: "variance_exploded",
                },
            ]))
        })
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 8, 5)))
        .await
        .unwrap_err();
    match err {
        EfeError::Divergence { rounds, reason } => {
            assert_eq!(rounds, 2);
            assert_eq!(reason, "variance_exploded");
        }
        other => panic!("expected divergence, got {other:?}"),
    }
}

// ---------- Timeout without convergence ----------

#[tokio::test]
async fn controller_reports_timeout_when_never_converging() {
    let dim = 4;
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("valid"), 4));
    let fusion = Arc::new(MockFusion::new(convergence_stats(4, dim as usize)));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, ""));
    let aligner = Arc::new(MockAligner::safe());

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .consensus_factory(|| Box::new(ScriptedDetector::new(vec![Verdict::Continue; 10])))
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 4, 4)))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        EfeError::ConvergenceTimeout { rounds: 4, .. }
    ));
}

// ---------- Mode collapse recovery ----------

#[tokio::test]
async fn controller_recovers_from_mode_collapse() {
    let dim = 4;
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("valid"), 20));
    let fusion = Arc::new(MockFusion::new(convergence_stats(3, dim as usize)));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, "recovered"));
    let aligner = Arc::new(MockAligner::safe());
    let telemetry = Arc::new(RecordingTelemetry::default());

    // Round 1: collapse → recovery.
    // Round 2: continue.
    // Round 3: converged.
    let verdicts = vec![
        Verdict::ModeCollapse,
        Verdict::Continue,
        Verdict::Converged { confidence: 0.88 },
    ];

    let controller = SwarmController::builder()
        .genesis(genesis.clone())
        .fusion(fusion.clone())
        .dissolver(dissolver)
        .alignment(aligner)
        .telemetry(telemetry.clone())
        .consensus_factory(move || Box::new(ScriptedDetector::new(verdicts.clone())))
        .build()
        .expect("build");

    let resp = controller
        .execute(Request::for_tests(spec(dim, 20, 8)))
        .await
        .expect("recovered");
    assert_eq!(resp.rounds_executed, 3);

    // Recovery triggered:
    assert_eq!(fusion.noise_calls.lock().expect("lock").len(), 1);
    let respawns = genesis.respawn_calls.lock().expect("lock");
    assert_eq!(respawns.len(), 1);
    assert_eq!(respawns[0], 4); // 20 * 0.2 = 4

    let events = telemetry.events.lock().expect("lock");
    assert!(events.iter().any(|e| e.contains("mode_collapse_recovery")));
}

#[tokio::test]
async fn controller_gives_up_after_max_collapse_retries() {
    let dim = 4;
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("valid"), 10));
    let fusion = Arc::new(MockFusion::new(convergence_stats(10, dim as usize)));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, ""));
    let aligner = Arc::new(MockAligner::safe());

    // Policy allows 2 retries, so after 3 collapses we give up.
    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .consensus_factory(|| Box::new(ScriptedDetector::new(vec![Verdict::ModeCollapse; 10])))
        .config(ControllerConfig::default())
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 10, 5)))
        .await
        .unwrap_err();
    assert!(matches!(err, EfeError::ModeCollapse { .. }));
}

// ---------- Alignment breach ----------

#[tokio::test]
async fn controller_fails_on_alignment_breach() {
    let dim = 4;
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("valid"), 6));
    let fusion = Arc::new(MockFusion::new(convergence_stats(1, dim as usize)));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, "unsafe"));
    let aligner = Arc::new(MockAligner::breaching()); // delta=0.99 > max=0.15

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged {
                confidence: 0.9,
            }]))
        })
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 6, 3)))
        .await
        .unwrap_err();
    match err {
        EfeError::AlignmentBreach { delta, max } => {
            assert!((delta - 0.99).abs() < 1e-6);
            assert!((max - 0.15).abs() < 1e-6);
        }
        other => panic!("expected alignment breach, got {other:?}"),
    }
}

// ---------- Numerical explosion ----------

#[tokio::test]
async fn controller_aborts_on_numerical_explosion() {
    let dim = 4;
    let mut bad_stats = FusionStats::synthetic_converging(1, dim as usize);
    bad_stats.has_nan = true;

    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("valid"), 4));
    let fusion = Arc::new(MockFusion::new(vec![bad_stats]));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, ""));
    let aligner = Arc::new(MockAligner::safe());

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .consensus_factory(|| Box::new(ScriptedDetector::new(vec![Verdict::Continue])))
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 4, 3)))
        .await
        .unwrap_err();
    assert!(matches!(err, EfeError::NumericalExplosion { round: 1 }));
}

// ---------- Budget exhaustion ----------

#[tokio::test]
async fn controller_aborts_on_wall_clock_budget() {
    // Genesis that sleeps long enough to exhaust a tight budget.
    struct SlowGenesis {
        inner: MockGenesis,
    }
    #[async_trait::async_trait]
    impl efe_core::swarm::GenesisStrategy for SlowGenesis {
        async fn spawn(
            &self,
            spec: &efe_core::types::SwarmSpec,
        ) -> Result<
            (
                efe_core::memory::NanoAgentBatch,
                efe_core::els::EpistemicLatentSpace,
            ),
            EfeError,
        > {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            self.inner.spawn(spec).await
        }
        async fn respawn_diversify(
            &self,
            batch: &mut efe_core::memory::NanoAgentBatch,
            count: u32,
        ) -> Result<(), EfeError> {
            self.inner.respawn_diversify(batch, count).await
        }
    }

    let dim = 4;
    let genesis = Arc::new(SlowGenesis {
        inner: MockGenesis::new(LatentDim::new(dim).expect("valid"), 4),
    });
    let fusion = Arc::new(MockFusion::new(convergence_stats(1, dim as usize)));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, ""));
    let aligner = Arc::new(MockAligner::safe());

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged {
                confidence: 0.9,
            }]))
        })
        .build()
        .expect("build");

    // 5ms budget: genesis sleeps 20ms → arbiter.check() after spawn fails.
    let mut req = Request::for_tests(spec(dim, 4, 5));
    req.spec.budget.max_wall_clock_ms = 5;

    let err = controller.execute(req).await.unwrap_err();
    assert!(matches!(
        err,
        EfeError::BudgetExhausted { kind: "wall_clock" }
    ));
}

// ---------- Dissolver failure ----------

#[tokio::test]
async fn controller_propagates_dissolver_error() {
    let dim = 4;
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("valid"), 4));
    let fusion = Arc::new(MockFusion::new(convergence_stats(1, dim as usize)));
    let dissolver = Arc::new(FailingDissolver);
    let aligner = Arc::new(MockAligner::safe());

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged {
                confidence: 0.8,
            }]))
        })
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 4, 3)))
        .await
        .unwrap_err();
    assert!(matches!(err, EfeError::KernelFailure(_)));
}

// ---------- Invalid spec ----------

#[tokio::test]
async fn controller_rejects_invalid_spec() {
    let dim = 4;
    let mut spec = spec(dim, 4, 3);
    // Corrupt the spec manually: embedding is too short.
    spec.problem_embedding.truncate(2);

    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("valid"), 4));
    let fusion = Arc::new(MockFusion::new(convergence_stats(1, dim as usize)));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, ""));
    let aligner = Arc::new(MockAligner::safe());

    let controller = SwarmController::builder()
        .genesis(genesis.clone())
        .fusion(fusion.clone())
        .dissolver(dissolver)
        .alignment(aligner)
        .consensus_factory(|| Box::new(ScriptedDetector::new(vec![Verdict::Continue])))
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec))
        .await
        .unwrap_err();
    assert!(matches!(err, EfeError::InvalidConfig(_)));

    // Ensure genesis was NOT called.
    assert_eq!(*genesis.spawn_calls.lock().expect("lock"), 0);
    assert_eq!(*fusion.round_calls.lock().expect("lock"), 0);
}

// ---------- Concurrency ----------

#[tokio::test]
async fn controller_handles_concurrent_requests() {
    let dim = 4;
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("valid"), 4));
    let fusion = Arc::new(MockFusion::new(convergence_stats(1, dim as usize)));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, "ok"));
    let aligner = Arc::new(MockAligner::safe());

    let controller = SwarmController::builder()
        .genesis(genesis.clone())
        .fusion(fusion.clone())
        .dissolver(dissolver)
        .alignment(aligner)
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged {
                confidence: 0.95,
            }]))
        })
        .build()
        .expect("build");

    let mut handles = vec![];
    for _ in 0..20 {
        let c = controller.clone_handle();
        let req = Request::for_tests(spec(dim, 4, 3));
        handles.push(tokio::spawn(async move { c.execute(req).await }));
    }
    for h in handles {
        let resp = h.await.expect("join").expect("success");
        assert_eq!(resp.rounds_executed, 1);
    }
    // All 20 swarms executed.
    assert_eq!(*genesis.spawn_calls.lock().expect("lock"), 20);
}
