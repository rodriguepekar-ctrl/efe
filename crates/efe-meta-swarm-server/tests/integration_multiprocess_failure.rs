//! PR #10 — multi-process failure-mode tests.
//!
//! Injects realistic failure modes at the syscall level :
//!   * participant unreachable (bogus address in
//!     `EFE_META_SWARM_PARTICIPANTS`) ;
//!   * participant SIGKILL-ed before the call (TCP refused on
//!     reconnect) ;
//!   * controller manifest declares an id that's **not** in
//!     `EFE_META_SWARM_PARTICIPANTS` (split-brain manifest, T18).
//!
//! Tests covered :
//!   * `iterate_fails_unavailable_when_participant_unreachable`  (AC5 + AC9)
//!   * `iterate_fails_when_participant_killed_before_call`        (AC5 + AC10)
//!   * `controller_with_manifest_mismatch_fails_at_iterate`       (AC5 + AC11)

#![cfg(unix)]

mod common;

use std::time::Duration;

use common::multiprocess::{connect_controller, ProcessHarness};
use efe_meta_swarm_server::manifest::ParticipantManifest;
use tonic::Request;

/// AC9 : controller declares an absent participant in its
/// `EFE_META_SWARM_PARTICIPANTS` env var with a bogus address.
/// Iterate fails at `RemoteParticipant::connect` with
/// `Code::Unavailable` (Discovery error).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_fails_unavailable_when_participant_unreachable() {
    // Spawn 2 alive participants + 1 absent (bogus addr).
    let harness = ProcessHarness::spawn_kovant_with_missing("spend-categorization").await;
    // Wait only for the alive ones (the harness's `participants` Vec
    // excludes the absent one).
    harness
        .wait_for_ready(Duration::from_secs(5))
        .await
        .expect("controller + 2 alive participants ready");

    let mut client = connect_controller(harness.controller_endpoint()).await;
    let err = client
        .iterate(Request::new(common::iterate_request(8, 100, 1.0e-3)))
        .await
        .unwrap_err();

    assert_eq!(
        err.code(),
        tonic::Code::Unavailable,
        "expected UNAVAILABLE ; got {:?} (message : {})",
        err.code(),
        err.message(),
    );
    assert!(
        err.message().contains("connect to spend-categorization")
            || err.message().contains("connect"),
        "unexpected error message : {}",
        err.message(),
    );
}

/// AC10 : a healthy topology, then a SIGKILL on one participant
/// before any Iterate.  Subsequent Iterate fails cleanly with
/// `Unavailable` (connect refused) or `Internal` (RST mid-call) —
/// both are valid recoverable states.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_fails_when_participant_killed_before_call() {
    let mut harness = ProcessHarness::spawn_kovant().await;
    harness
        .wait_for_ready(Duration::from_secs(5))
        .await
        .expect("ready");

    // SIGKILL one participant.
    assert!(harness.sigkill_participant("contract-risk"));
    let status = harness
        .wait_participant("contract-risk", Duration::from_secs(2))
        .await
        .expect("participant exited");
    // SIGKILL exits with code 137 (128 + 9) on Linux.  We don't
    // assert the exact code — `status.success()` is enough to know
    // it terminated.
    assert!(!status.success(), "expected non-zero exit ; got {status:?}");

    // Give the OS a moment to release the TCP port.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut client = connect_controller(harness.controller_endpoint()).await;
    let err = client
        .iterate(Request::new(common::iterate_request(8, 100, 1.0e-3)))
        .await
        .unwrap_err();

    assert!(
        matches!(err.code(), tonic::Code::Unavailable | tonic::Code::Internal),
        "unexpected code : {:?} (message : {})",
        err.code(),
        err.message(),
    );
}

/// AC11 : controller's manifest declares 3 ids ; only 2 are in
/// `EFE_META_SWARM_PARTICIPANTS`.  At Iterate time, the controller's
/// `lookup_endpoints` (PR #8) fails for the absent manifest id.
///
/// This is the first end-to-end exercise of threat **T18**
/// (split-brain manifest).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn controller_with_manifest_mismatch_fails_at_iterate() {
    let rows = vec![
        ParticipantManifest {
            id: "alpha".into(),
            dim: 8,
            kappa: 0.6,
            seed: 1,
            blas_backend: "none".into(),
        },
        ParticipantManifest {
            id: "beta".into(),
            dim: 8,
            kappa: 0.7,
            seed: 2,
            blas_backend: "none".into(),
        },
        ParticipantManifest {
            id: "ghost".into(),
            dim: 8,
            kappa: 0.5,
            seed: 3,
            blas_backend: "none".into(),
        },
    ];
    // Manifest declares alpha + beta + ghost ; only alpha and beta
    // are spawned + listed in EFE_META_SWARM_PARTICIPANTS.
    let harness = ProcessHarness::spawn_partial_kovant(rows, &["alpha", "beta"]).await;
    harness
        .wait_for_ready(Duration::from_secs(5))
        .await
        .expect("alpha + beta + controller ready");

    let mut client = connect_controller(harness.controller_endpoint()).await;
    let err = client
        .iterate(Request::new(common::iterate_request(8, 100, 1.0e-3)))
        .await
        .unwrap_err();

    assert_eq!(
        err.code(),
        tonic::Code::Unavailable,
        "expected UNAVAILABLE for split-brain manifest ; got {:?}",
        err.code(),
    );
    assert!(
        err.message().contains("no static_participants entry"),
        "expected 'no static_participants entry' in message ; got {}",
        err.message(),
    );
    assert!(
        err.message().contains("ghost"),
        "expected mention of the missing id 'ghost' ; got {}",
        err.message(),
    );
}
