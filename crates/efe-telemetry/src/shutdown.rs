//! Helpers around graceful shutdown of the telemetry stack.
//!
//! The main entry point is [`crate::TelemetryHandle::shutdown`]; this
//! module exposes lower-level utilities used by the init pipeline and
//! by callers that want to drive their own shutdown choreography.

use std::time::Duration;

use tokio::time::timeout;

/// Run `fut` with a shutdown budget. Returns `true` if the future
/// completed within `budget`, `false` on timeout.
pub async fn graceful_flush<F>(budget: Duration, fut: F) -> bool
where
    F: std::future::Future<Output = ()>,
{
    timeout(budget, fut).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_true_when_future_resolves_before_timeout() {
        let ok = graceful_flush(Duration::from_millis(100), async {}).await;
        assert!(ok);
    }

    #[tokio::test]
    async fn returns_false_when_future_exceeds_timeout() {
        let ok = graceful_flush(
            Duration::from_millis(10),
            tokio::time::sleep(Duration::from_millis(100)),
        )
        .await;
        assert!(!ok);
    }
}
