//! PR #10 — controller × N participants happy-path e2e over real
//! OS subprocesses.
//!
//! These tests close the "process-boundary" gap left by PR #9 :
//! before this file, every e2e test ran controller + participants
//! in **one** process (shared tokio runtime, shared address space).
//! Here, each binary is a real OS subprocess communicating over
//! `127.0.0.1` gRPC.
//!
//! Tests covered :
//!   * `iterate_e2e_kovant_converges_across_real_processes` (AC4 + AC7)
//!   * `iterate_e2e_bit_identical_3_reruns_over_wire`       (AC4 + AC8)
//!   * `controller_info_lists_all_participants`             (AC4)
//!   * `participant_capabilities_reflects_manifest_across_processes` (AC4)

#![cfg(unix)]

mod common;

use std::time::Duration;

use common::multiprocess::{connect_controller, connect_participant, ProcessHarness};
use efe_meta_swarm_proto::meta_swarm::{
    CapabilitiesRequest, InfoRequest, ResetRequest, StopReason as ProtoStopReason,
};
use tonic::Request;

/// AC4 + AC7 : Kovant scenario converges across 4 OS subprocesses.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_e2e_kovant_converges_across_real_processes() {
    let harness = ProcessHarness::spawn_kovant().await;
    harness
        .wait_for_ready(Duration::from_secs(5))
        .await
        .expect("harness ready");

    let mut client = connect_controller(harness.controller_endpoint()).await;
    let resp = client
        .iterate(Request::new(common::iterate_request(8, 100, 1.0e-3)))
        .await
        .expect("iterate")
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
    assert_eq!(resp.final_states.len(), 3);
}

/// AC4 + AC8 : 3 reruns over real gRPC are bit-identical.  The
/// gold-standard determinism check for Phase 2.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_e2e_bit_identical_3_reruns_over_wire() {
    let harness = ProcessHarness::spawn_kovant().await;
    harness
        .wait_for_ready(Duration::from_secs(5))
        .await
        .expect("ready");

    let mut client = connect_controller(harness.controller_endpoint()).await;

    let mut sigs: Vec<(u32, u64)> = Vec::new();
    for i in 0..3 {
        client
            .reset(Request::new(ResetRequest::default()))
            .await
            .unwrap_or_else(|e| panic!("reset failed on run {i} : {e}"));

        let resp = client
            .iterate(Request::new(common::iterate_request(8, 100, 1.0e-3)))
            .await
            .unwrap_or_else(|e| panic!("iterate failed on run {i} : {e}"))
            .into_inner();

        sigs.push((resp.iterations, resp.final_residual.to_bits()));
    }

    assert!(
        sigs.windows(2).all(|w| w[0] == w[1]),
        "non-deterministic reruns across real process boundary : {sigs:?}",
    );
}

/// AC4 : the controller's `Info` snapshot lists all manifest
/// participants over real gRPC.  Validates that `state.info_snapshot`
/// (PR #8) populates correctly from a manifest loaded by a separate
/// process.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn controller_info_lists_all_participants() {
    let harness = ProcessHarness::spawn_kovant().await;
    harness
        .wait_for_ready(Duration::from_secs(5))
        .await
        .expect("ready");

    let mut client = connect_controller(harness.controller_endpoint()).await;
    let info = client
        .info(Request::new(InfoRequest {}))
        .await
        .expect("info")
        .into_inner();

    assert_eq!(info.proto_version, "v1");
    assert_eq!(info.configured_participants.len(), 3);
    let ids: Vec<&str> = info
        .configured_participants
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    assert!(ids.contains(&"supplier-scoring"), "ids = {ids:?}");
    assert!(ids.contains(&"contract-risk"), "ids = {ids:?}");
    assert!(ids.contains(&"spend-categorization"), "ids = {ids:?}");
}

/// AC4 : participant's `Capabilities` reflects its manifest row,
/// observed by a direct gRPC call to the participant subprocess
/// (no controller in the loop).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn participant_capabilities_reflects_manifest_across_processes() {
    let harness = ProcessHarness::spawn_kovant().await;
    harness
        .wait_for_ready(Duration::from_secs(5))
        .await
        .expect("ready");

    let endpoint = harness
        .participant_endpoint("contract-risk")
        .expect("contract-risk endpoint");
    let mut client = connect_participant(endpoint).await;
    let cap = client
        .capabilities(Request::new(CapabilitiesRequest {}))
        .await
        .expect("capabilities")
        .into_inner();
    let desc = cap.descriptor.expect("descriptor present");
    assert_eq!(desc.id, "contract-risk");
    assert_eq!(desc.dim, 8);
    assert!((desc.kappa - 0.7).abs() < 1e-6);
    assert_eq!(desc.seed, 22);
    assert!(desc.features.iter().any(|f| f == "determinism"));
    assert!(desc.features.iter().any(|f| f == "step_index"));
}
