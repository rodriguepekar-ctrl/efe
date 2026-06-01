//! PR #10 — multi-process shutdown / drain tests.
//!
//! Exercises the SIGTERM-driven graceful drain path of both binaries.
//! These tests are the most timing-sensitive in PR #10 ; they accept
//! a generous set of outcomes (any of OK / Cancelled / Unavailable /
//! DeadlineExceeded) provided the process exits within the
//! `drain_timeout + slack` window.
//!
//! Tests covered :
//!   * `controller_sigterm_drains_inflight_iterate`            (AC6 + AC12)
//!   * `participant_sigterm_then_controller_iterate_fails_cleanly` (AC6 + AC13)

#![cfg(unix)]

mod common;

use std::time::{Duration, Instant};

use common::multiprocess::{connect_controller, ProcessHarness, DRAIN_TIMEOUT_PLUS_SLACK};
use efe_meta_swarm_proto::meta_swarm::StopReason as ProtoStopReason;
use tonic::Request;

/// AC12 (crown jewel) : start an Iterate, SIGTERM the controller
/// mid-flight, observe that the controller drains within
/// `DRAIN_TIMEOUT_PLUS_SLACK` and that the in-flight Iterate either
/// completes normally or returns a clean error code.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn controller_sigterm_drains_inflight_iterate() {
    let mut harness = ProcessHarness::spawn_kovant().await;
    harness
        .wait_for_ready(Duration::from_secs(5))
        .await
        .expect("ready");

    let endpoint = harness.controller_endpoint();

    // Kick off an Iterate in a tokio task ; it returns Result or panics
    // on transport errors.
    let endpoint_for_task = endpoint.clone();
    let iterate_task = tokio::spawn(async move {
        let mut client = connect_controller(endpoint_for_task).await;
        client
            .iterate(Request::new(common::iterate_request(8, 100, 1.0e-3)))
            .await
    });

    // Give the Iterate a 50 ms head start so SIGTERM is genuinely
    // "mid-flight".
    tokio::time::sleep(Duration::from_millis(50)).await;

    let sigterm_at = Instant::now();
    harness.sigterm_controller();

    // Await the Iterate ; allow any of OK / Cancelled / Unavailable /
    // DeadlineExceeded — all valid outcomes for a controller-side
    // drain mid-iteration.
    let result = iterate_task.await.expect("iterate task joined");
    match result {
        Ok(resp) => {
            let r = resp.into_inner();
            // Drained quickly enough to converge.
            assert_eq!(
                r.stop_reason,
                ProtoStopReason::Converged as i32,
                "successful iterate but stop_reason = {:?}",
                r.stop_reason,
            );
        }
        Err(s) => {
            assert!(
                matches!(
                    s.code(),
                    tonic::Code::Cancelled
                        | tonic::Code::Unavailable
                        | tonic::Code::DeadlineExceeded
                        | tonic::Code::Internal,
                ),
                "unexpected error code on SIGTERM drain : {:?} (message : {})",
                s.code(),
                s.message(),
            );
        }
    }

    // Controller must exit within the drain window.
    let status = harness
        .wait_controller(DRAIN_TIMEOUT_PLUS_SLACK)
        .await
        .expect("controller exited within drain timeout");
    let elapsed = sigterm_at.elapsed();

    assert!(
        elapsed < DRAIN_TIMEOUT_PLUS_SLACK,
        "drain took {elapsed:?} > {DRAIN_TIMEOUT_PLUS_SLACK:?}",
    );
    assert!(
        status.success(),
        "controller exit status : {status:?} (drain should be clean)",
    );
}

/// AC13 : SIGTERM a participant, wait for drain, then call Iterate on
/// the controller.  Expect a clean error (Unavailable on connect or
/// Internal mid-call).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn participant_sigterm_then_controller_iterate_fails_cleanly() {
    let mut harness = ProcessHarness::spawn_kovant().await;
    harness
        .wait_for_ready(Duration::from_secs(5))
        .await
        .expect("ready");

    // SIGTERM one participant ; wait for it to drain (it has no
    // in-flight work so this is fast).
    assert!(harness.sigterm_participant("supplier-scoring"));
    let status = harness
        .wait_participant("supplier-scoring", DRAIN_TIMEOUT_PLUS_SLACK)
        .await
        .expect("participant drained");
    assert!(
        status.success(),
        "participant exit status : {status:?} (drain should be clean)",
    );

    // Now Iterate against the controller ; it tries to connect to a
    // listener that no longer exists.
    let mut client = connect_controller(harness.controller_endpoint()).await;
    let err = client
        .iterate(Request::new(common::iterate_request(8, 100, 1.0e-3)))
        .await
        .unwrap_err();

    assert!(
        matches!(err.code(), tonic::Code::Unavailable | tonic::Code::Internal),
        "unexpected error code after participant drain : {:?} (message : {})",
        err.code(),
        err.message(),
    );
}
