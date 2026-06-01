//! Unix signal handling.
//!
//! PR #6 handles SIGTERM (graceful drain) + SIGINT (Ctrl+C, same as
//! SIGTERM).  PR #12 adds SIGHUP (manifest reload) via
//! [`install_sighup_handler`].  PR #18 will add SIGUSR1 (dump debug
//! to stderr).
//!
//! ## SIGHUP design
//!
//! [`install_sighup_handler`] installs a background tokio task that
//! listens for SIGHUP forever and calls `callback` once per signal.
//! Concurrent SIGHUPs are **coalesced** through a
//! [`tokio::sync::Notify`] — the handler wakes the worker once,
//! subsequent SIGHUPs that arrive while the worker is mid-flight are
//! merged into a single follow-up reload at the end (cf.
//! `EFE_A1_PHASE2_PR12_ARCHITECTURE.md` § 8.3 and risk PR12-R2).
//!
//! The returned `JoinHandle` lives for the binary's lifetime ; teardown
//! happens implicitly when the tokio runtime drops on shutdown.  Note
//! that the **listener task** keeps a separate `JoinHandle` we
//! intentionally drop — its only role is to bump the Notify, and a
//! panic there is non-recoverable anyway (would mean the libc layer
//! broke).

use std::future::Future;
use std::sync::Arc;

use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::Notify;

use tracing::info;

/// Wait for SIGTERM or SIGINT, whichever comes first.
///
/// Returns a `&'static str` identifying the signal (for logging).
pub async fn await_termination() -> &'static str {
    let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT handler");

    tokio::select! {
        _ = sigterm.recv() => {
            info!("received SIGTERM");
            "SIGTERM"
        }
        _ = sigint.recv() => {
            info!("received SIGINT");
            "SIGINT"
        }
    }
}

/// Install a SIGHUP handler that fires `callback` once per signal,
/// coalescing back-to-back signals into a single follow-up reload.
///
/// Returns the `JoinHandle` of the **worker** task ; the listener task
/// is also spawned but its handle is discarded (architecture spec
/// § 8.2 ; the listener has no state and is non-recoverable on panic).
///
/// # Concurrency contract
///
///   * Exactly **one** `callback` future runs at a time.
///   * A SIGHUP that arrives **while the callback is running** is
///     queued via `Notify::notify_one` and triggers exactly **one**
///     follow-up callback when the current run finishes.
///   * `N` concurrent SIGHUPs (`N ≥ 2`) collapse to at most **one
///     in-flight + one queued** callback (PR12-R2 / § 8.3).
///
/// # `cfg(unix)` gating
///
/// SIGHUP is Unix-only.  On non-Unix targets (macOS dev laptops are
/// Unix-flavored ; Windows CI is the case worth caring about) the
/// non-Unix arm returns a `JoinHandle` of a future that never resolves,
/// keeping the rest of `bin/controller.rs` compilable.
///
/// # Panics
///
/// `signal(SignalKind::hangup())` panics on Unix only if the process
/// has already exhausted its signal-handler slots — a system-level
/// configuration error.  Matches `await_termination`'s `expect`.
#[cfg(unix)]
pub fn install_sighup_handler<F, Fut>(callback: F) -> tokio::task::JoinHandle<()>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let notify = Arc::new(Notify::new());

    // Listener : every SIGHUP bumps the Notify.  Multi-shot — runs
    // for the binary's lifetime.
    let notify_signal = Arc::clone(&notify);
    tokio::spawn(async move {
        let mut sighup = signal(SignalKind::hangup()).expect("install SIGHUP handler");
        while sighup.recv().await.is_some() {
            info!("SIGHUP received ; scheduling reload");
            notify_signal.notify_one();
        }
    });

    // Worker : awaits the Notify, runs the callback.  Coalescing is
    // automatic — `Notify::notify_one` collapses multiple bumps that
    // arrive while no waiter is parked into a single permit, so the
    // next `notified()` resolves once even after N signals.
    let notify_worker = Arc::clone(&notify);
    tokio::spawn(async move {
        loop {
            notify_worker.notified().await;
            callback().await;
        }
    })
}

/// Non-Unix shim : SIGHUP doesn't exist, so we return a task that
/// never resolves and never fires the callback.  Production
/// deployment target is Linux server (Phase-2 architecture § AD11) ;
/// this branch exists purely so `cargo check --target x86_64-pc-windows-msvc`
/// stays green.
#[cfg(not(unix))]
pub fn install_sighup_handler<F, Fut>(_callback: F) -> tokio::task::JoinHandle<()>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async {
        std::future::pending::<()>().await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Smoke-only : the installer returns a `JoinHandle`, doesn't
    /// panic, and the callback is not invoked spuriously without
    /// a signal.
    ///
    /// Signal-raising coverage lives in `tests/sighup_handler_smoke.rs`
    /// (separate crate, allowed to use the unsafe `libc::raise` /
    /// `libc::kill` interfaces that this lib forbids).
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn install_sighup_handler_returns_join_handle_without_panic() {
        let count = Arc::new(AtomicU32::new(0));
        let count_for_cb = Arc::clone(&count);
        let handle = install_sighup_handler(move || {
            let c = Arc::clone(&count_for_cb);
            async move {
                c.fetch_add(1, Ordering::SeqCst);
            }
        });
        // No signal raised : callback must NOT have been invoked.
        // Give the listener a beat to wire up.
        tokio::time::sleep(std::time::Duration::from_millis(75)).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "callback must not fire without a SIGHUP being raised",
        );
        // Worker task is still alive.
        assert!(
            !handle.is_finished(),
            "worker task must keep running for the binary's lifetime",
        );
        // Abort to free resources cleanly.
        handle.abort();
    }
}
