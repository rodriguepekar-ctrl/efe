//! PR #9 — controller × N real participants e2e.
//!
//! Replaces the `TestParticipantSvc` stand-in from PR #8.  Three real
//! `ParticipantService` instances + one in-process `ControllerService`.
//! Asserts :
//!   * convergence (procurement-style 3-participant topology)
//!   * bit-identical determinism across 3 reruns (AC8)

mod common;

use efe_meta_swarm_proto::meta_swarm::{ResetRequest, StopReason as ProtoStopReason};
use efe_meta_swarm_server::controller::ControllerService;
use efe_meta_swarm_server::proto::MetaSwarmService;
use tonic::Request;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_e2e_kovant_converges_with_real_participants() {
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_e2e_bit_identical_3_reruns() {
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
