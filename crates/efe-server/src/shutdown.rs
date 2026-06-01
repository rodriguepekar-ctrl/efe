//! Graceful shutdown plumbing.
//!
//! [`Shutdown`] can be triggered manually (tests) or by a SIGTERM /
//! SIGINT on Unix. It exposes a `notified()` future that
//! [`tonic::transport::Server::serve_with_shutdown`] can await.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tokio::sync::Notify;

/// Handle to a shutdown signal shared between the server and any
/// subsystem that needs to know we are draining (e.g. the health
/// service flipping to `NOT_SERVING`).
#[derive(Clone, Debug)]
pub struct Shutdown {
    triggered: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

impl Shutdown {
    /// Construct a new handle.
    #[must_use]
    pub fn new() -> Self {
        Self {
            triggered: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Trigger shutdown manually.
    pub fn trigger(&self) {
        self.triggered.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Returns `true` if the shutdown has been requested.
    #[must_use]
    pub fn is_triggered(&self) -> bool {
        self.triggered.load(Ordering::SeqCst)
    }

    /// Awaitable future that completes when shutdown is requested,
    /// either through [`Self::trigger`] or an OS signal (Unix only).
    pub async fn notified(&self) {
        if self.is_triggered() {
            return;
        }
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = match signal(SignalKind::terminate()) {
                Ok(s) => s,
                Err(_) => {
                    self.notify.notified().await;
                    return;
                }
            };
            let mut sigint = match signal(SignalKind::interrupt()) {
                Ok(s) => s,
                Err(_) => {
                    self.notify.notified().await;
                    return;
                }
            };
            tokio::select! {
                _ = self.notify.notified() => {}
                _ = sigterm.recv() => {}
                _ = sigint.recv() => {}
            }
        }
        #[cfg(not(unix))]
        {
            self.notify.notified().await;
        }
        self.triggered.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn manual_trigger_propagates() {
        let s = Shutdown::new();
        assert!(!s.is_triggered());
        let s2 = s.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            s2.trigger();
        });
        s.notified().await;
        assert!(s.is_triggered());
    }

    #[tokio::test]
    async fn is_triggered_shortcircuits_notified() {
        let s = Shutdown::new();
        s.trigger();
        s.notified().await; // returns immediately.
        assert!(s.is_triggered());
    }

    #[test]
    fn default_is_not_triggered() {
        let s = Shutdown::default();
        assert!(!s.is_triggered());
    }
}
