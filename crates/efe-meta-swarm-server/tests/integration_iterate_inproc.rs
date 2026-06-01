//! PR #8 — in-process smoke tests for the wired Iterate / Reset
//! orchestration.
//!
//! PR #9 update : the inline `TestParticipantSvc` stand-in is gone ;
//! these tests now spawn the **real** `ParticipantService` via the
//! shared `common::spawn_participant_simple` helper.  The semantics
//! and pass-criteria are unchanged from PR #8.
//!
//! Tests covered :
//!   * `iterate_kovant_three_participants_converges`
//!   * `iterate_bit_identical_3_reruns`
//!   * `reset_fan_out_succeeds`
//!
//! Inflight rejection is exercised in the unit tests in
//! `controller/service.rs` (deterministic — no timing race).

mod common;

use efe_meta_swarm_proto::meta_swarm::{ResetRequest, StopReason as ProtoStopReason};
use efe_meta_swarm_server::controller::ControllerService;
use efe_meta_swarm_server::proto::MetaSwarmService;
use tonic::Request;

/// AC4 + AC5 : 3 participants converge through the gRPC stack.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_kovant_three_participants_converges() {
    let dim = 8;
    let (a1, _h1) = common::spawn_participant_simple("supplier-scoring", dim as u32, 0.6, 11).await;
    let (a2, _h2) = common::spawn_participant_simple("contract-risk", dim as u32, 0.7, 22).await;
    let (a3, _h3) =
        common::spawn_participant_simple("spend-categorization", dim as u32, 0.5, 33).await;

    let statics = vec![
        ("supplier-scoring".into(), dim, 0.6_f32, 11_u64, a1),
        ("contract-risk".into(), dim, 0.7_f32, 22_u64, a2),
        ("spend-categorization".into(), dim, 0.5_f32, 33_u64, a3),
    ];
    let state = common::build_controller_state_for(&statics, 10, dim);
    let svc = ControllerService::new(state);

    let resp = svc
        .iterate(Request::new(common::iterate_request(
            dim as u32, 100, 1.0e-3,
        )))
        .await
        .expect("iterate succeeds")
        .into_inner();

    assert_eq!(
        resp.stop_reason,
        ProtoStopReason::Converged as i32,
        "expected Converged ; got resp = {resp:?}",
    );
    assert!(
        resp.iterations <= 80,
        "iterations = {} ; expected <= 80",
        resp.iterations,
    );
    assert!(
        resp.final_residual <= 1.0e-3,
        "final_residual = {} ; expected <= 1e-3",
        resp.final_residual,
    );
    assert!(resp.meta_consensus.is_some(), "meta_consensus missing");
    assert_eq!(resp.final_states.len(), 3, "expected 3 final states");
    for fs in &resp.final_states {
        assert_eq!(fs.dim as usize, dim);
        assert_eq!(fs.data.len(), dim);
    }
}

/// AC8 : 3 reruns of the same iterate with reset between produce
/// bit-identical results.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_bit_identical_3_reruns() {
    let dim = 8;
    let (a1, _h1) = common::spawn_participant_simple("p1", dim as u32, 0.6, 11).await;
    let (a2, _h2) = common::spawn_participant_simple("p2", dim as u32, 0.7, 22).await;

    let statics = vec![
        ("p1".into(), dim, 0.6_f32, 11_u64, a1),
        ("p2".into(), dim, 0.7_f32, 22_u64, a2),
    ];
    let state = common::build_controller_state_for(&statics, 10, dim);
    let svc = ControllerService::new(state);

    let mut signatures = Vec::new();
    for i in 0..3 {
        svc.reset(Request::new(ResetRequest::default()))
            .await
            .unwrap_or_else(|e| panic!("reset failed on run {i}: {e}"));

        let resp = svc
            .iterate(Request::new(common::iterate_request(
                dim as u32, 100, 1.0e-3,
            )))
            .await
            .unwrap_or_else(|e| panic!("iterate failed on run {i}: {e}"))
            .into_inner();

        signatures.push((resp.iterations, resp.final_residual.to_bits()));
    }

    assert!(
        signatures.windows(2).all(|w| w[0] == w[1]),
        "non-deterministic reruns: {signatures:?}",
    );
}

/// `Reset` fans out to all participants successfully.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reset_fan_out_succeeds() {
    let dim = 4;
    let (a1, _h1) = common::spawn_participant_simple("p1", dim as u32, 0.6, 11).await;
    let (a2, _h2) = common::spawn_participant_simple("p2", dim as u32, 0.7, 22).await;
    let (a3, _h3) = common::spawn_participant_simple("p3", dim as u32, 0.5, 33).await;

    let statics = vec![
        ("p1".into(), dim, 0.6_f32, 11_u64, a1),
        ("p2".into(), dim, 0.7_f32, 22_u64, a2),
        ("p3".into(), dim, 0.5_f32, 33_u64, a3),
    ];
    let state = common::build_controller_state_for(&statics, 10, dim);
    let svc = ControllerService::new(state);

    let resp = svc
        .reset(Request::new(ResetRequest::default()))
        .await
        .expect("reset succeeds")
        .into_inner();

    // PR #8 returns reset_gen=0 ; PR #12 will track real generations.
    assert_eq!(resp.reset_gen, 0);
}
