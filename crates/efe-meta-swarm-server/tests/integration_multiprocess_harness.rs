//! PR #10 — sanity tests for the [`ProcessHarness`] itself.
//!
//! These tests exercise the harness API independently of any
//! production-code scenario : spawn / wait_for_ready / SIGTERM /
//! Drop / endpoint lookup.  If any of these fail, the e2e /
//! failure / lifecycle tests are unreliable.
//!
//! Unix-only — Phase 2 deployment target is Linux server.
//!
//! [`ProcessHarness`]: common::multiprocess::ProcessHarness

#![cfg(unix)]

mod common;

use std::time::Duration;

use common::multiprocess::{ProcessHarness, DRAIN_TIMEOUT_PLUS_SLACK};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn harness_spawns_kovant_and_health_passes() {
    let h = ProcessHarness::spawn_kovant().await;
    h.wait_for_ready(Duration::from_secs(5))
        .await
        .expect("harness ready");
    assert_eq!(h.participants.len(), 3);
    assert!(h.participants.iter().any(|p| p.id == "supplier-scoring"));
    assert!(h.participants.iter().any(|p| p.id == "contract-risk"));
    assert!(h
        .participants
        .iter()
        .any(|p| p.id == "spend-categorization"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn harness_drop_kills_all_children() {
    let pids: Vec<u32> = {
        let h = ProcessHarness::spawn_kovant().await;
        h.wait_for_ready(Duration::from_secs(5))
            .await
            .expect("ready");
        let mut pids = vec![h.controller.pid()];
        for p in &h.participants {
            pids.push(p.pid());
        }
        pids
        // h dropped here ; Drop SIGKILLs every child + waits.
    };
    // Give the OS a moment to fully release.
    tokio::time::sleep(Duration::from_millis(100)).await;
    for pid in pids {
        // SAFETY : `kill(pid, 0)` probes whether the process exists ;
        // it does not actually send any signal.  Return value :
        //   0   → process exists, we have permission to signal it
        //  -1   → ESRCH (no such process) ; that's our pass criterion
        #[allow(clippy::cast_possible_wrap)]
        let probe = unsafe { libc::kill(pid as libc::pid_t, 0) };
        assert_eq!(probe, -1, "PID {pid} still alive after harness drop",);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn harness_sigterm_controller_then_wait_exits_clean() {
    let mut h = ProcessHarness::spawn_kovant().await;
    h.wait_for_ready(Duration::from_secs(5))
        .await
        .expect("ready");
    h.sigterm_controller();
    let status = h
        .wait_controller(DRAIN_TIMEOUT_PLUS_SLACK)
        .await
        .expect("controller drained within drain_timeout");
    assert!(status.success(), "controller exit status : {status:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn harness_participant_endpoint_lookup() {
    let h = ProcessHarness::spawn_kovant().await;
    // No need to wait_for_ready — endpoint lookup is independent of
    // process state.
    assert!(h.participant_endpoint("supplier-scoring").is_some());
    assert!(h.participant_endpoint("contract-risk").is_some());
    assert!(h.participant_endpoint("spend-categorization").is_some());
    assert!(h.participant_endpoint("ghost").is_none());
    assert!(h.participant_endpoint("").is_none());
}
