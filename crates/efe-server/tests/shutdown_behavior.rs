//! Drain + shutdown behaviour tests (M12.3).
//!
//! The `Shutdown` type owns the cooperative-cancel primitive; these
//! tests focus on the exposed *semantics* (idempotence, wake-everyone,
//! no-op-after-trigger) because the process-signal integration is
//! intrinsically tied to Unix and best covered by the existing
//! `integration_grpc.rs` suite that exercises the full server.
//!
//! M12 §25 acceptance criteria covered:
//! * CA-M12-02 (exit clean in ≤ 30 s) — timing sanity checks.
//! * CA-M12-03 (readiness probes flip) — exercised via runtime.rs
//!   wiring (`health_reporter.set_not_serving` on shutdown).
//!   That integration path is tested end-to-end in `integration_grpc.rs`.

use std::time::{Duration, Instant};

use efe_server::Shutdown;

#[tokio::test]
async fn manual_trigger_completes_notified_in_under_100ms() {
    // Budget check: a manual trigger on an idle Shutdown must wake
    // `notified()` essentially instantly. We allow 100 ms to absorb
    // scheduler jitter on congested CI machines.
    let s = Shutdown::new();
    let started = Instant::now();
    let s2 = s.clone();
    tokio::spawn(async move {
        // A short sleep keeps the assertion meaningful — without it,
        // `trigger()` could race ahead of the `notified()` await and
        // make the check trivial.
        tokio::time::sleep(Duration::from_millis(5)).await;
        s2.trigger();
    });
    s.notified().await;
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "shutdown took too long: {:?}",
        started.elapsed()
    );
    assert!(s.is_triggered());
}

#[tokio::test]
async fn trigger_is_idempotent() {
    // An operator double-tapping SIGTERM (or the signal handler
    // receiving SIGTERM + SIGINT within the same tick) must not
    // produce observable misbehaviour. The internal flag stays true
    // and no additional work is scheduled.
    let s = Shutdown::new();
    s.trigger();
    s.trigger();
    s.trigger();
    assert!(s.is_triggered());
    // `notified()` on an already-triggered shutdown resolves
    // immediately; this is the readiness-probe hot path.
    let start = Instant::now();
    s.notified().await;
    assert!(start.elapsed() < Duration::from_millis(20));
}

#[tokio::test]
async fn multiple_awaiters_all_wake_up() {
    // The `Shutdown` primitive uses `Notify::notify_waiters`, so all
    // concurrent awaiters must resume. Regression guard against a
    // future change to `notify_one`.
    let s = Shutdown::new();
    let mut joins = Vec::new();
    for _ in 0..8 {
        let s2 = s.clone();
        joins.push(tokio::spawn(async move {
            s2.notified().await;
            Instant::now()
        }));
    }
    // Give all awaiters a chance to park.
    tokio::time::sleep(Duration::from_millis(5)).await;
    let trigger_instant = Instant::now();
    s.trigger();
    for j in joins {
        let woke_at = j.await.expect("task");
        assert!(
            woke_at >= trigger_instant,
            "awaiter woke BEFORE trigger (impossible)"
        );
        assert!(
            woke_at.duration_since(trigger_instant) < Duration::from_millis(50),
            "awaiter did not wake promptly: delta={:?}",
            woke_at.duration_since(trigger_instant)
        );
    }
    assert!(s.is_triggered());
}

#[tokio::test]
async fn notified_after_trigger_short_circuits() {
    // A new awaiter on a shutdown that has already fired gets an
    // immediate Ready — no spurious wait. This is what the health
    // service relies on to flip NOT_SERVING without delay.
    let s = Shutdown::new();
    s.trigger();
    let start = Instant::now();
    s.notified().await;
    assert!(start.elapsed() < Duration::from_millis(20));
}

#[test]
fn clone_shares_state() {
    // Explicit proof that `.clone()` returns a handle to the same
    // underlying atomic + Notify — not an independent Shutdown.
    let s = Shutdown::new();
    let s2 = s.clone();
    assert!(!s2.is_triggered());
    s.trigger();
    assert!(s2.is_triggered());
}

#[test]
fn default_handle_is_not_triggered() {
    let s = Shutdown::default();
    assert!(!s.is_triggered());
}
