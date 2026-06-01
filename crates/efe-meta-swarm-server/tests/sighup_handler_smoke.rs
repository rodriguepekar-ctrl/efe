//! PR #12 — SIGHUP handler in-process smoke tests.
//!
//! These tests raise SIGHUP against the current process and assert the
//! [`install_sighup_handler`] callback fires.  They live in an
//! integration test crate (not the lib) so the `unsafe { libc::raise }`
//! call is permitted — the production lib has `#![forbid(unsafe_code)]`.
//!
//! Architecture spec § 8.4 — three coverage goals :
//!
//!   1. `sighup_handler_fires_callback_once_per_signal`
//!   2. `concurrent_sighups_are_coalesced`
//!   3. `handler_keeps_listening_after_callback_returns`
//!
//! Risk PR12-R2 (`Notify::notify_one` semantics surprise) is exercised
//! by test 2.
//!
//! ## Why `#[serial]`
//!
//! SIGHUP is a process-wide signal : every handler registered via
//! `tokio::signal::unix::signal(SignalKind::hangup())` receives every
//! SIGHUP delivered to the process.  When two of these tests run on
//! different threads at the same time, test A's `libc::raise` wakes
//! BOTH test A's worker AND test B's worker — coalescing counts
//! become non-deterministic.  `serial_test::serial` ensures the
//! three tests run one at a time so the kernel's signal queue is
//! observed by exactly one test instance.
//!
//! [`install_sighup_handler`]: efe_meta_swarm_server::observability::shutdown::install_sighup_handler

#![cfg(unix)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use efe_meta_swarm_server::observability::shutdown::install_sighup_handler;
use serial_test::serial;

/// Send SIGHUP to the current process.  Async-signal-safe ; the libc
/// call returns immediately and the kernel queues the signal.
fn raise_sighup_to_self() {
    // SAFETY : `libc::raise` is async-signal-safe.  Sending SIGHUP to
    // our own PID is harmless — the installed handler bumps a Notify,
    // no surprising side-effect.  We never call `raise` from inside
    // a Drop or unwinding panic, so the libc invariant holds.
    #[allow(unsafe_code)]
    unsafe {
        libc::raise(libc::SIGHUP);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial(sighup)]
async fn sighup_handler_fires_callback_once_per_signal() {
    let count = Arc::new(AtomicU32::new(0));
    let count_for_cb = Arc::clone(&count);
    let _worker = install_sighup_handler(move || {
        let c = Arc::clone(&count_for_cb);
        async move {
            c.fetch_add(1, Ordering::SeqCst);
        }
    });

    // Give the listener task a beat to install the kernel handler.
    tokio::time::sleep(Duration::from_millis(75)).await;
    raise_sighup_to_self();

    // Poll for the bump.  Signal → Notify → worker chain is typically
    // < 1 ms but loaded CI workers can be slow.
    for _ in 0..80 {
        if count.load(Ordering::SeqCst) >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        count.load(Ordering::SeqCst) >= 1,
        "callback must have fired at least once after one SIGHUP",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial(sighup)]
async fn concurrent_sighups_are_coalesced() {
    // Architecture § 8.3 / risk PR12-R2 :
    //   * the worker runs the callback while parked in a long sleep ;
    //   * 3 SIGHUPs arrive during that window ;
    //   * Notify::notify_one collapses them to ≤ 1 follow-up.
    //
    // Tolerance : we accept 1 or 2 total runs (one in-flight + one
    // queued is the canonical worst case).  3 or more would be a
    // coalescing bug.
    let runs = Arc::new(AtomicU32::new(0));
    let runs_for_cb = Arc::clone(&runs);

    let _worker = install_sighup_handler(move || {
        let c = Arc::clone(&runs_for_cb);
        async move {
            c.fetch_add(1, Ordering::SeqCst);
            // Hold the worker so subsequent SIGHUPs queue up.
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    });

    tokio::time::sleep(Duration::from_millis(75)).await;
    // Burst of 3 SIGHUPs, tightly packed.
    for _ in 0..3 {
        raise_sighup_to_self();
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // Wait long enough for the in-flight + the coalesced follow-up to
    // land.  200 ms per run × 2 = 400 ms + slack.
    tokio::time::sleep(Duration::from_millis(700)).await;

    let final_runs = runs.load(Ordering::SeqCst);
    assert!(
        (1..=2).contains(&final_runs),
        "expected 1 or 2 coalesced callback runs after 3 SIGHUPs ; got {final_runs} \
         (3+ would mean Notify::notify_one is not coalescing)",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial(sighup)]
async fn handler_keeps_listening_after_callback_returns() {
    // Multi-shot guarantee : after the first reload finishes, the
    // worker loops back to `notified()` and accepts the next SIGHUP.
    let runs = Arc::new(AtomicU32::new(0));
    let runs_for_cb = Arc::clone(&runs);
    let _worker = install_sighup_handler(move || {
        let c = Arc::clone(&runs_for_cb);
        async move {
            c.fetch_add(1, Ordering::SeqCst);
        }
    });

    tokio::time::sleep(Duration::from_millis(75)).await;

    // First SIGHUP.
    raise_sighup_to_self();
    for _ in 0..80 {
        if runs.load(Ordering::SeqCst) >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let after_first = runs.load(Ordering::SeqCst);
    assert!(after_first >= 1, "first SIGHUP must fire callback");

    // Wait for the worker to park again.
    tokio::time::sleep(Duration::from_millis(75)).await;

    // Second SIGHUP.
    raise_sighup_to_self();
    for _ in 0..80 {
        if runs.load(Ordering::SeqCst) > after_first {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        runs.load(Ordering::SeqCst) > after_first,
        "second SIGHUP must also fire callback (multi-shot)",
    );
}
