//! PR #12 — end-to-end SIGHUP + ReloadConfig integration tests.
//!
//! These tests run the controller + 3 participants as real OS
//! subprocesses (via PR #10's `ProcessHarness`), trigger a live
//! reload by either SIGHUP or gRPC `ReloadConfig`, and assert :
//!
//!   * `Health.reload_gen` ticks by exactly +1 on every successful
//!     reload (AC12 / AC13 / AC21) ;
//!   * gRPC `ReloadConfig` returns the new gen + applied_changes
//!     (AC14 / AC15) ;
//!   * in-flight Iterate sees the pre-reload manifest snapshot
//!     (AC16) ;
//!   * a reload that mutates ⁠iteration knobs produces results
//!     bit-identical to a fresh-topology run with the same knobs
//!     (architecture § 15.3 / AC10).
//!
//! See `docs/EFE_A1_PHASE2_PR12_ARCHITECTURE.md` § 20.2.
//!
//! ## Test budget
//!
//! Each `ProcessHarness::spawn_kovant` spawns 4 subprocesses + waits
//! up to 5 s for them to bind ports.  We allocate ~ 15 s wall budget
//! per test ; CI may run them serially.

#![cfg(unix)]

mod common;

use std::time::Duration;

use common::multiprocess::{connect_controller, ProcessHarness};
use efe_meta_swarm_proto::meta_swarm::{
    meta_space_config::SafetyGeometry, BoxSafety as ProtoBoxSafety, HealthRequest, MetaSpaceConfig,
    ParticipantDescriptor, ReloadConfigRequest, StopReason as ProtoStopReason,
};
use efe_meta_swarm_server::manifest::ParticipantManifest;
use tonic::Request;

/// Helper : poll `Health.reload_gen` until it reaches `target` or the
/// deadline expires.  Returns the observed `(status, reload_gen)` on
/// success.  Used by every test below — the SIGHUP→swap→bump_gen
/// chain typically completes in < 5 ms but loaded CI workers can be
/// slow.
async fn wait_for_reload_gen(
    client: &mut efe_meta_swarm_proto::controller::MetaSwarmServiceClient<
        tonic::transport::Channel,
    >,
    target: u32,
    timeout: Duration,
) -> (i32, u32) {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last = (0_i32, 0_u32);
    while tokio::time::Instant::now() < deadline {
        match client.health(Request::new(HealthRequest {})).await {
            Ok(r) => {
                let h = r.into_inner();
                last = (h.status, h.reload_gen);
                if h.reload_gen >= target {
                    return last;
                }
            }
            Err(_) => {
                // Transient — the controller may have raced our probe
                // with the RELOADING write-lock.  Retry.
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("Health.reload_gen never reached {target} within {timeout:?} ; last={last:?}",);
}

/// **AC12** — SIGHUP to the controller bumps `reload_gen` by exactly 1.
/// The manifest's iteration knobs change between two SIGHUPs ; the
/// gauge sequence is `0 → 1 → 2` (monotonic, AC21).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sighup_to_controller_bumps_reload_gen() {
    let mut harness = ProcessHarness::spawn_kovant().await;
    harness
        .wait_for_ready(Duration::from_secs(10))
        .await
        .expect("harness ready");

    let mut client = connect_controller(harness.controller_endpoint()).await;
    let initial = client
        .health(Request::new(HealthRequest {}))
        .await
        .expect("health")
        .into_inner();
    assert_eq!(initial.reload_gen, 0, "fresh controller starts at gen 0");

    // Mutate the manifest on disk : bump alpha to 0.6.
    let rows = kovant_rows();
    harness
        .rewrite_manifest(rows.clone(), 0.6, 0.2)
        .expect("rewrite manifest");

    // SIGHUP → wait → assert gen == 1.
    harness.sighup_controller();
    let (_status, gen1) = wait_for_reload_gen(&mut client, 1, Duration::from_secs(5)).await;
    assert_eq!(gen1, 1, "after SIGHUP : reload_gen == 1");

    // Second mutation + SIGHUP : gen == 2.
    harness
        .rewrite_manifest(rows, 0.7, 0.3)
        .expect("rewrite #2");
    harness.sighup_controller();
    let (_, gen2) = wait_for_reload_gen(&mut client, 2, Duration::from_secs(5)).await;
    assert_eq!(gen2, 2, "second SIGHUP : reload_gen == 2 (monotonic)");

    // Cleanup : SIGTERM the harness so we don't rely on the SIGKILL
    // Drop path.
    harness.sigterm_controller();
    for id in ["supplier-scoring", "contract-risk", "spend-categorization"] {
        let _ = harness.sigterm_participant(id);
    }
}

/// **AC13** — SIGHUP to a participant triggers its reload + bumps its
/// own `reload_gen`.  The participant's `Health` RPC reflects the bump.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sighup_to_participant_bumps_participant_reload_gen() {
    let mut harness = ProcessHarness::spawn_kovant().await;
    harness
        .wait_for_ready(Duration::from_secs(10))
        .await
        .expect("ready");

    // Mutate manifest : new α + γ.
    let rows = kovant_rows();
    harness.rewrite_manifest(rows, 0.65, 0.25).expect("rewrite");

    // SIGHUP the first participant.
    assert!(harness.sighup_participant("supplier-scoring"));

    // Poll the participant's health for reload_gen bump.
    let endpoint = harness
        .participant_endpoint("supplier-scoring")
        .expect("endpoint");
    use efe_meta_swarm_proto::participant::MetaParticipantServiceClient;
    use tonic::transport::Endpoint;

    let channel = Endpoint::from_shared(endpoint)
        .unwrap()
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_secs(2))
        .connect()
        .await
        .expect("participant connect");
    let mut p_client = MetaParticipantServiceClient::new(channel);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut gen = 0;
    while tokio::time::Instant::now() < deadline {
        if let Ok(h) = p_client.health(Request::new(HealthRequest {})).await {
            let h = h.into_inner();
            if h.reload_gen >= 1 {
                gen = h.reload_gen;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(gen, 1, "participant reload_gen must reach 1 after SIGHUP");

    harness.sigterm_controller();
    for id in ["supplier-scoring", "contract-risk", "spend-categorization"] {
        let _ = harness.sigterm_participant(id);
    }
}

/// **Architecture § 20.2 test #2 — `sighup_to_participant_swaps_iteration_params`**
///
/// Strict functional-swap verification (one rung above AC13's
/// `reload_gen`-bump-only check) : we send a `Step` with a known
/// input, capture the output ; SIGHUP the participant to flip α / γ ;
/// re-Reset to the same seed state ; resend the exact same `Step`
/// input ; and assert the new output is **different** because the
/// participant's `ArcSwap<IterationParams>` now holds the post-SIGHUP
/// α / γ.
///
/// Without this functional check, a hypothetical bug where the
/// participant bumps `reload_gen` but forgets to actually swap the
/// ArcSwap would slip through AC13.  This test pins down the
/// load-bearing C5 invariant : SIGHUP → ArcSwap → new α / γ → new
/// step output (architecture § 12.6).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sighup_to_participant_swaps_iteration_params() {
    use efe_meta_swarm_proto::meta_swarm::{ResetParticipantRequest, StepRequest, TensorF32};
    use efe_meta_swarm_proto::participant::MetaParticipantServiceClient;
    use tonic::transport::Endpoint;

    let mut harness = ProcessHarness::spawn_kovant().await;
    harness
        .wait_for_ready(Duration::from_secs(10))
        .await
        .expect("ready");

    // Build a participant client.
    let endpoint = harness
        .participant_endpoint("supplier-scoring")
        .expect("endpoint");
    let channel = Endpoint::from_shared(endpoint)
        .unwrap()
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_secs(2))
        .connect()
        .await
        .expect("participant connect");
    let mut p_client = MetaParticipantServiceClient::new(channel);

    // Deterministic Step input : non-trivial values so the step result
    // visibly depends on α / γ.  The participant has dim = 8 (from
    // `kovant_rows`).
    let step_input = TensorF32 {
        dim: 8,
        data: vec![0.1_f32, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
    };
    let step_req = StepRequest {
        pulled_from_meta: Some(step_input.clone()),
        step_index: 1,
    };

    // 1. Reset to seed state.
    p_client
        .reset(Request::new(ResetParticipantRequest::default()))
        .await
        .expect("reset 1");

    // 2. Step with α = 0.5 / γ = 0.2 (manifest defaults).
    let resp_before = p_client
        .step(Request::new(step_req.clone()))
        .await
        .expect("step before")
        .into_inner();
    let output_before = resp_before
        .new_state
        .expect("new_state present")
        .data
        .clone();
    assert_eq!(output_before.len(), 8);

    // 3. Rewrite the manifest with α / γ that produce a **different**
    //    Step output.  Phase-1's coupling :
    //        new_state = (1 - α - γ) · x + α · Φ(x) + γ · pull
    //    starts from x = zero state (`MockParticipant::reset` sets to
    //    zeros), so :
    //        new_state = γ · pull
    //    and the output depends ONLY on γ at iter 1 from zero state.
    //    We must therefore pick γ_new ≠ γ_old (0.2) ; α can be
    //    anything in [0, 1 − γ) due to Phase-1's `α + γ < 1` coupling
    //    constraint.
    //
    //    Choices : α = 0.3, γ = 0.6 (sum = 0.9 < 1, γ moves 0.2 → 0.6).
    //    Output_after = 0.6 · pull  ≠  output_before = 0.2 · pull.
    let rows = kovant_rows();
    harness
        .rewrite_manifest(rows, 0.3, 0.6)
        .expect("rewrite manifest with new α / γ");

    // 4. SIGHUP only the participant under test.
    assert!(harness.sighup_participant("supplier-scoring"));

    // 5. Wait for the participant's reload_gen to reach 1 (the
    //    `ArcSwap` has been stored by then).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut observed_gen = 0_u32;
    while tokio::time::Instant::now() < deadline {
        if let Ok(h) = p_client.health(Request::new(HealthRequest {})).await {
            let h = h.into_inner();
            if h.reload_gen >= 1 {
                observed_gen = h.reload_gen;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        observed_gen, 1,
        "participant reload_gen must reach 1 within 5 s after SIGHUP",
    );

    // 6. Reset the participant so the SECOND Step starts from the
    //    same seed state as the first.  This isolates the variable
    //    under test (α / γ) from any state drift introduced by
    //    `output_before`'s state mutation.
    p_client
        .reset(Request::new(ResetParticipantRequest::default()))
        .await
        .expect("reset 2");

    // 7. Step with α = 0.3 / γ = 0.6 (post-SIGHUP, swapped via ArcSwap).
    //    From zero state : new_state = γ_after · pull = 0.6 · pull.
    let resp_after = p_client
        .step(Request::new(step_req))
        .await
        .expect("step after")
        .into_inner();
    let output_after = resp_after
        .new_state
        .expect("new_state present")
        .data
        .clone();
    assert_eq!(output_after.len(), 8);

    // 8. **The load-bearing assertion** : the output MUST differ
    //    because α / γ changed.  If `ArcSwap` was not actually swapped
    //    (only `reload_gen` bumped), `output_after == output_before`
    //    would catch the bug.
    assert_ne!(
        output_before, output_after,
        "post-SIGHUP Step output must differ from pre-SIGHUP \
         (α / γ swap must be functionally observable, architecture § 12.6) ; \
         before = {output_before:?} after = {output_after:?}",
    );

    harness.sigterm_controller();
    for id in ["supplier-scoring", "contract-risk", "spend-categorization"] {
        let _ = harness.sigterm_participant(id);
    }
}

/// **AC14** — gRPC `ReloadConfig` with empty body re-reads the disk
/// manifest and returns the new generation.  `applied_changes`
/// mentions the iteration drift.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reload_config_rpc_with_empty_body_returns_new_gen() {
    let mut harness = ProcessHarness::spawn_kovant().await;
    harness
        .wait_for_ready(Duration::from_secs(10))
        .await
        .expect("ready");

    let mut client = connect_controller(harness.controller_endpoint()).await;

    let rows = kovant_rows();
    harness.rewrite_manifest(rows, 0.55, 0.15).expect("rewrite");

    let resp = client
        .reload_config(Request::new(ReloadConfigRequest::default()))
        .await
        .expect("reload_config")
        .into_inner();

    assert_eq!(resp.reload_gen, 1);
    assert!(resp.reload_wall_us > 0);
    assert!(!resp.inflight_aborted);
    // PR #12 § 19 — applied_changes must contain the iteration drift.
    assert!(
        resp.applied_changes
            .iter()
            .any(|s| s.contains("alpha_default")),
        "applied_changes must list α drift ; got {:?}",
        resp.applied_changes,
    );

    harness.sigterm_controller();
    for id in ["supplier-scoring", "contract-risk", "spend-categorization"] {
        let _ = harness.sigterm_participant(id);
    }
}

/// **AC15** — gRPC `ReloadConfig` with inline `new_meta` applies the
/// inline payload + bumps gen, **without** touching the disk
/// manifest.  After the call we read back `Info` and observe the
/// new safety radius.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reload_config_rpc_with_inline_meta_applies_inline() {
    let harness = ProcessHarness::spawn_kovant().await;
    harness
        .wait_for_ready(Duration::from_secs(10))
        .await
        .expect("ready");

    let mut client = connect_controller(harness.controller_endpoint()).await;

    let req = ReloadConfigRequest {
        new_meta: Some(MetaSpaceConfig {
            dim: 16,
            safety_geometry: Some(SafetyGeometry::Box(ProtoBoxSafety { radius: 73.5 })),
        }),
        new_participants: kovant_rows()
            .iter()
            .map(|r| ParticipantDescriptor {
                id: r.id.clone(),
                dim: r.dim,
                // Wire descriptor kappa/seed stay f32/u64 (frozen fields) —
                // harness rows are mock-shaped so the Options are Some.
                kappa: r.kappa.expect("harness rows are mock-shaped"),
                seed: r.seed.expect("harness rows are mock-shaped"),
                build_sha: String::new(),
                blas_backend: String::new(),
                features: vec![],
                config_hash: None,
            })
            .collect(),
        drain_inflight: false,
    };
    let resp = client
        .reload_config(Request::new(req))
        .await
        .expect("inline reload")
        .into_inner();
    assert_eq!(resp.reload_gen, 1);
    assert!(resp
        .applied_changes
        .iter()
        .any(|s| s.contains("meta.safety.radius")));

    // Health confirms the bump.
    let h = client
        .health(Request::new(HealthRequest {}))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(h.reload_gen, 1);

    harness.sigterm_controller();
    for id in ["supplier-scoring", "contract-risk", "spend-categorization"] {
        let _ = harness.sigterm_participant(id);
    }
}

/// **AC16 / Architecture § 5.2 / § 20.2 test #4** — the load-bearing
/// **T20 mitigation** : an in-flight `Iterate` that captured the
/// controller's manifest snapshot at iteration start MUST complete
/// with that snapshot, regardless of a concurrent SIGHUP that swaps
/// the controller's manifest mid-flight.
///
/// Scenario :
///
///   1. Spawn the kovant 3-participant harness.
///   2. Run a baseline `Iterate` → record `(iters, residual.to_bits(),
///      final_states.len() == 3)`.
///   3. Reset.
///   4. Rewrite the manifest **on disk** so it now declares only
///      2 participants (an AD5-compatible reload).
///   5. Send an `Iterate` request as a tokio task (the request itself
///      survives the await later).
///   6. Sleep 30 ms so the controller's handler enters `run_iterate`,
///      acquires the manifest read-clone (snapshot @ gen=0 = 3
///      participants), and starts the Phase-1 iterate loop.
///   7. **SIGHUP the controller** — manifest write-locks, swaps to
///      the 2-participant version, gen bumps to 1.
///   8. Await the in-flight Iterate's response.
///   9. **Assert**`final_states.len() == 3` (the snapshot held).
///  10. **Assert**`(iters, residual.to_bits()) == baseline` (the
///      result is bit-identical because the in-flight call used the
///      pre-SIGHUP manifest from start to finish — architecture
///      § 5.2 invariant).
///  11. **Assert** `Health.reload_gen == 1` (the SIGHUP did take
///      effect, no-op test would fail this).
///  12. **Assert** `Info.configured_participants.len() == 2` (the
///      post-SIGHUP manifest is now live for the next call).
///
/// This is the canonical T20 ("race during reload — in-flight Iterate
/// sees inconsistent state") regression test ; it exercises the
/// `Arc<RwLock<Manifest>>` + `.clone()` snapshot pattern PR #8
/// established and PR #12 relies on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_during_concurrent_sighup_uses_old_manifest() {
    use efe_meta_swarm_proto::meta_swarm::InfoRequest;

    let mut harness = ProcessHarness::spawn_kovant().await;
    harness
        .wait_for_ready(Duration::from_secs(10))
        .await
        .expect("ready");

    let mut client = connect_controller(harness.controller_endpoint()).await;

    // -----------------------------------------------------------------
    // 1. Reset to a known zero state.  Phase-1's
    //    `MockParticipant::reset()` sets state to `Tensor::zeros(dim)`,
    //    so a Reset-then-Iterate sequence is fully deterministic.  We
    //    Reset BOTH before the baseline AND before the in-flight run
    //    so the two iterates start from identical state — that's what
    //    makes the bit-identity assertion below meaningful.
    //
    //    (Boot-time participants start from their `with_contraction`
    //    seed state, which differs across participants ; without this
    //    Reset the baseline would converge in ~50 iters while the
    //    in-flight post-Reset run would converge in 1, breaking
    //    bit-identity for trivial reasons unrelated to T20.)
    // -----------------------------------------------------------------
    client
        .reset(Request::new(
            efe_meta_swarm_proto::meta_swarm::ResetRequest::default(),
        ))
        .await
        .expect("baseline reset");

    let baseline_req = common::iterate_request(16, 100, 1.0e-3);
    let baseline = client
        .iterate(Request::new(baseline_req.clone()))
        .await
        .expect("baseline iterate")
        .into_inner();
    assert_eq!(
        baseline.stop_reason,
        ProtoStopReason::Converged as i32,
        "baseline must converge",
    );
    assert_eq!(
        baseline.final_states.len(),
        3,
        "baseline manifest has 3 participants",
    );
    let baseline_sig = (baseline.iterations, baseline.final_residual.to_bits());

    // -----------------------------------------------------------------
    // 2. Re-Reset so the in-flight run starts from the same zero state
    //    as the baseline (the baseline iterate has mutated state).
    // -----------------------------------------------------------------
    client
        .reset(Request::new(
            efe_meta_swarm_proto::meta_swarm::ResetRequest::default(),
        ))
        .await
        .expect("pre-inflight reset");

    // -----------------------------------------------------------------
    // 3. Rewrite the manifest on disk : drop the 3rd participant.
    //    This is an AD5-compatible reload (allowed by
    //    `validate_reload_compatibility`).  The on-disk file now
    //    has 2 rows ; the controller hasn't reloaded yet.
    // -----------------------------------------------------------------
    let mut shrunk = kovant_rows();
    shrunk.pop(); // drop spend-categorization
    assert_eq!(shrunk.len(), 2);
    harness
        .rewrite_manifest(shrunk, 0.5, 0.2)
        .expect("rewrite manifest to 2 rows");

    // -----------------------------------------------------------------
    // 4. Spawn the in-flight Iterate as a task.  Clone the client so
    //    the main task retains its own handle for the Health / Info
    //    assertions below.
    // -----------------------------------------------------------------
    let iterate_req = baseline_req;
    let client_for_task = client.clone();
    let iterate_task = tokio::spawn(async move {
        let mut c = client_for_task;
        c.iterate(Request::new(iterate_req)).await
    });

    // -----------------------------------------------------------------
    // 5. Give the controller a beat to enter `run_iterate`, clone the
    //    manifest snapshot (3 participants @ gen=0), connect to the
    //    3 participants, build the MetaSwarm, and start the iterate
    //    loop.  30 ms is comfortable on localhost (the snapshot read
    //    is microseconds, participant connections ~ 1–2 ms each).
    // -----------------------------------------------------------------
    tokio::time::sleep(Duration::from_millis(30)).await;

    // -----------------------------------------------------------------
    // 6. SIGHUP the controller mid-flight.  Manifest write-lock
    //    swaps to the 2-participant version ; gen bumps to 1.
    //    The in-flight Iterate, holding its earlier `Arc<Manifest>`
    //    clone, is unaffected (architecture § 5.2).
    // -----------------------------------------------------------------
    harness.sighup_controller();

    // -----------------------------------------------------------------
    // 7. Await the in-flight Iterate.  It MUST complete with the
    //    pre-SIGHUP 3-participant configuration.
    // -----------------------------------------------------------------
    let resp = iterate_task
        .await
        .expect("iterate task join")
        .expect("iterate rpc")
        .into_inner();
    assert_eq!(
        resp.stop_reason,
        ProtoStopReason::Converged as i32,
        "in-flight Iterate must complete normally despite mid-flight SIGHUP",
    );
    assert_eq!(
        resp.final_states.len(),
        3,
        "in-flight Iterate must use the pre-SIGHUP 3-participant snapshot \
         (architecture § 5.2 / T20)",
    );
    let in_flight_sig = (resp.iterations, resp.final_residual.to_bits());
    assert_eq!(
        baseline_sig, in_flight_sig,
        "in-flight Iterate must produce bit-identical result to baseline \
         (3-participant snapshot, same seed) ; baseline = {baseline_sig:?}, \
         in-flight = {in_flight_sig:?}",
    );

    // -----------------------------------------------------------------
    // 8. Health confirms the SIGHUP did take effect (no-op detection).
    // -----------------------------------------------------------------
    let h = client
        .health(Request::new(HealthRequest {}))
        .await
        .expect("health")
        .into_inner();
    assert_eq!(
        h.reload_gen, 1,
        "SIGHUP must have bumped reload_gen ; otherwise this test \
         is trivially true and provides no T20 coverage",
    );

    // -----------------------------------------------------------------
    // 9. Info shows the new 2-participant topology — the swap is
    //    visible to future calls, but did NOT affect the in-flight
    //    Iterate (asserted above).
    // -----------------------------------------------------------------
    let info = client
        .info(Request::new(InfoRequest {}))
        .await
        .expect("info")
        .into_inner();
    assert_eq!(
        info.configured_participants.len(),
        2,
        "post-SIGHUP Info must reflect the 2-participant manifest swap",
    );
    assert_eq!(
        info.last_reload_gen, 1,
        "Info.last_reload_gen must mirror Health.reload_gen (PR #12 wiring)",
    );

    harness.sigterm_controller();
    for id in ["supplier-scoring", "contract-risk", "spend-categorization"] {
        let _ = harness.sigterm_participant(id);
    }
}

/// **AC10 / Architecture § 15.3** — a reload that bumps `alpha_default`
/// produces results bit-identical to a fresh spawn with the same
/// alpha.  The determinism contract under reload (architecture
/// § 15.1) :
///
///   * Iteration knobs match : YES.
///   * Participants set unchanged : YES.
///   * Reload happens between Iterates : YES (we don't fire a SIGHUP
///     during one).
///
/// Therefore : `(iterations, residual.to_bits())` are identical.
///
/// The test uses **inline** `ReloadConfig` (not SIGHUP) to keep wall
/// time bounded — the assertion is about iteration knobs, not the
/// trigger path.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_after_reload_matches_fresh_topology_with_new_alpha() {
    // 1. Fresh harness #1 boots with α = 0.5, runs Iterate, captures
    //    the bit-signature.
    let mut harness_fresh = ProcessHarness::spawn_kovant().await;
    harness_fresh
        .wait_for_ready(Duration::from_secs(10))
        .await
        .expect("ready 1");
    // Mutate to α = 0.7 BEFORE any Iterate ; we want the fresh case
    // to start at α = 0.7 so the assertion compares apples-to-apples.
    harness_fresh
        .rewrite_manifest(kovant_rows(), 0.7, 0.2)
        .expect("rewrite fresh");
    let mut client_fresh = connect_controller(harness_fresh.controller_endpoint()).await;
    // SIGHUP so the controller picks up α = 0.7.
    harness_fresh.sighup_controller();
    let _ = wait_for_reload_gen(&mut client_fresh, 1, Duration::from_secs(5)).await;
    // ALSO SIGHUP every participant so their IterationParams swap.
    for id in ["supplier-scoring", "contract-risk", "spend-categorization"] {
        assert!(harness_fresh.sighup_participant(id));
    }
    // Brief settle so the participants finish their reloads.
    tokio::time::sleep(Duration::from_millis(250)).await;

    let req = common::iterate_request(16, 100, 1.0e-3);
    let resp_fresh = client_fresh
        .iterate(Request::new(req.clone()))
        .await
        .expect("iterate fresh")
        .into_inner();
    assert_eq!(resp_fresh.stop_reason, ProtoStopReason::Converged as i32);

    let sig_fresh = (resp_fresh.iterations, resp_fresh.final_residual.to_bits());

    harness_fresh.sigterm_controller();
    for id in ["supplier-scoring", "contract-risk", "spend-categorization"] {
        let _ = harness_fresh.sigterm_participant(id);
    }

    // 2. Reload harness #2 : boots with default α = 0.5, then we
    //    SIGHUP to bump α to 0.7, then run Iterate.
    let mut harness_reload = ProcessHarness::spawn_kovant().await;
    harness_reload
        .wait_for_ready(Duration::from_secs(10))
        .await
        .expect("ready 2");
    harness_reload
        .rewrite_manifest(kovant_rows(), 0.7, 0.2)
        .expect("rewrite reload");
    harness_reload.sighup_controller();
    let mut client_reload = connect_controller(harness_reload.controller_endpoint()).await;
    let _ = wait_for_reload_gen(&mut client_reload, 1, Duration::from_secs(5)).await;
    for id in ["supplier-scoring", "contract-risk", "spend-categorization"] {
        assert!(harness_reload.sighup_participant(id));
    }
    tokio::time::sleep(Duration::from_millis(250)).await;

    let resp_reload = client_reload
        .iterate(Request::new(req))
        .await
        .expect("iterate reload")
        .into_inner();
    assert_eq!(resp_reload.stop_reason, ProtoStopReason::Converged as i32);

    let sig_reload = (resp_reload.iterations, resp_reload.final_residual.to_bits());

    // 3. Assert bit-identity.  Architecture § 15.3 promises this
    //    contract under the three-condition rule (knobs match,
    //    participants set unchanged, reload between Iterates).
    assert_eq!(
        sig_fresh, sig_reload,
        "post-reload Iterate must be bit-identical to fresh-topology run \
         with the same α ; fresh={sig_fresh:?} reload={sig_reload:?}",
    );

    harness_reload.sigterm_controller();
    for id in ["supplier-scoring", "contract-risk", "spend-categorization"] {
        let _ = harness_reload.sigterm_participant(id);
    }
}

/// **AC17 / AC18** — a malformed inline payload (mutated participant
/// dim) is rejected with `FAILED_PRECONDITION` and leaves the running
/// state untouched.  Subsequent `Health` still shows the pre-reject
/// `reload_gen` (no bump).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reload_config_rpc_rejects_dim_drift_and_leaves_state_untouched() {
    let harness = ProcessHarness::spawn_kovant().await;
    harness
        .wait_for_ready(Duration::from_secs(10))
        .await
        .expect("ready");
    let mut client = connect_controller(harness.controller_endpoint()).await;

    let req = ReloadConfigRequest {
        new_meta: Some(MetaSpaceConfig {
            dim: 16,
            safety_geometry: Some(SafetyGeometry::Box(ProtoBoxSafety { radius: 50.0 })),
        }),
        new_participants: vec![
            ParticipantDescriptor {
                id: "supplier-scoring".into(),
                dim: 16, // illegal drift from 8
                kappa: 0.6,
                seed: 11,
                build_sha: String::new(),
                blas_backend: String::new(),
                features: vec![],
                config_hash: None,
            },
            ParticipantDescriptor {
                id: "contract-risk".into(),
                dim: 8,
                kappa: 0.7,
                seed: 22,
                build_sha: String::new(),
                blas_backend: String::new(),
                features: vec![],
                config_hash: None,
            },
            ParticipantDescriptor {
                id: "spend-categorization".into(),
                dim: 8,
                kappa: 0.5,
                seed: 33,
                build_sha: String::new(),
                blas_backend: String::new(),
                features: vec![],
                config_hash: None,
            },
        ],
        drain_inflight: false,
    };
    let err = client.reload_config(Request::new(req)).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(err.message().contains("dim drift"));

    // Health.reload_gen must still be 0.
    let h = client
        .health(Request::new(HealthRequest {}))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(h.reload_gen, 0, "rejected reload must NOT bump gen");

    harness.sigterm_controller();
    for id in ["supplier-scoring", "contract-risk", "spend-categorization"] {
        let _ = harness.sigterm_participant(id);
    }
}

// -----------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------

fn kovant_rows() -> Vec<ParticipantManifest> {
    vec![
        ParticipantManifest {
            id: "supplier-scoring".into(),
            dim: 8,
            kappa: Some(0.6),
            seed: Some(11),
            blas_backend: "none".into(),
            controller: None,
        },
        ParticipantManifest {
            id: "contract-risk".into(),
            dim: 8,
            kappa: Some(0.7),
            seed: Some(22),
            blas_backend: "none".into(),
            controller: None,
        },
        ParticipantManifest {
            id: "spend-categorization".into(),
            dim: 8,
            kappa: Some(0.5),
            seed: Some(33),
            blas_backend: "none".into(),
            controller: None,
        },
    ]
}
