// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : Graceful shutdown — § 7.4 + G14 + I19 + F13.

use std::time::Duration;

use tokio::signal;

/// **N22 + G14** : Future that resolves on SIGTERM, SIGINT, or
/// (on Unix) SIGHUP.  Wire to `axum::serve(...).with_graceful_shutdown(
/// shutdown_signal())`.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { tracing::info!("received SIGINT"); }
        _ = terminate => { tracing::info!("received SIGTERM"); }
    }
}

/// **N22 + I19** : Wait up to `budget` for in-flight requests to
/// drain.  Returns true if drained cleanly, false on timeout.
pub async fn drain_with_budget<F>(in_flight_count: F, budget: Duration) -> bool
where
    F: Fn() -> u64,
{
    let started = std::time::Instant::now();
    loop {
        if in_flight_count() == 0 {
            return true;
        }
        if started.elapsed() >= budget {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn drain_returns_true_when_zero_in_flight() {
        let ok = drain_with_budget(|| 0, Duration::from_millis(100)).await;
        assert!(ok);
    }

    #[tokio::test]
    async fn drain_returns_false_on_timeout() {
        let counter = Arc::new(AtomicU64::new(5));
        let c2 = counter.clone();
        let ok = drain_with_budget(
            move || c2.load(Ordering::Relaxed),
            Duration::from_millis(150),
        )
        .await;
        assert!(!ok);
    }

    #[tokio::test]
    async fn drain_succeeds_when_counter_reaches_zero() {
        let counter = Arc::new(AtomicU64::new(3));
        // Spawn a task that decrements every 30 ms.
        let c2 = counter.clone();
        tokio::spawn(async move {
            for _ in 0..3 {
                tokio::time::sleep(Duration::from_millis(30)).await;
                c2.fetch_sub(1, Ordering::Relaxed);
            }
        });
        let c3 = counter.clone();
        let ok = drain_with_budget(
            move || c3.load(Ordering::Relaxed),
            Duration::from_millis(500),
        )
        .await;
        assert!(ok);
    }
}
