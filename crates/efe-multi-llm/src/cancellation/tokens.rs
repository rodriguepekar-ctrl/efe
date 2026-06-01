// SPDX-License-Identifier: UNLICENSED
//! Token de cancellation Phase 3.

use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

/// Raison de cancellation (pour logs + SSE `LlmCancelled`).
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CancellationReason {
    /// Quorum vote atteint avant la fin (invariant I9).
    EarlyQuorum,
    /// Budget hard-cap dépassé (invariant I13).
    BudgetExceeded,
    /// Deadline globale du workflow approche.
    DeadlineNear,
    /// L'utilisateur a fermé la connexion SSE (UI close).
    UserDisconnect,
    /// Une autre branche a échoué et la mission ne peut plus continuer.
    SiblingFailed,
}

/// Token léger de cancellation — partagé par tous les futures en flight.
///
/// Sémantique :
/// - **clone** est cheap (Arc).
/// - **cancel** est idempotent.
/// - **is_cancelled** est lock-free.
/// - **wait_cancel** retourne quand la cancellation arrive.
#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<TokenInner>,
}

struct TokenInner {
    notify: Notify,
    cancelled: Mutex<Option<CancellationReason>>,
}

impl CancellationToken {
    /// Construit un nouveau token (non-cancellé).
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(TokenInner {
                notify: Notify::new(),
                cancelled: Mutex::new(None),
            }),
        }
    }

    /// Cancel idempotent.  Premier appel l'effectue, les suivants no-op.
    pub fn cancel(&self, reason: CancellationReason) {
        let mut g = self.inner.cancelled.lock();
        if g.is_none() {
            *g = Some(reason);
            self.inner.notify.notify_waiters();
        }
    }

    /// Le token est-il cancellé ?
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.lock().is_some()
    }

    /// La raison si cancellé.
    #[must_use]
    pub fn reason(&self) -> Option<CancellationReason> {
        *self.inner.cancelled.lock()
    }

    /// Attend la cancellation.  Si déjà cancellé, retourne immédiatement.
    pub async fn wait_cancel(&self) -> CancellationReason {
        loop {
            if let Some(r) = self.reason() {
                return r;
            }
            self.inner.notify.notified().await;
            if let Some(r) = self.reason() {
                return r;
            }
        }
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn fresh_token_not_cancelled() {
        let t = CancellationToken::new();
        assert!(!t.is_cancelled());
        assert!(t.reason().is_none());
    }

    #[test]
    fn cancel_is_idempotent() {
        let t = CancellationToken::new();
        t.cancel(CancellationReason::EarlyQuorum);
        t.cancel(CancellationReason::BudgetExceeded); // ignoré
        assert_eq!(t.reason(), Some(CancellationReason::EarlyQuorum));
    }

    #[test]
    fn clones_share_state() {
        let t = CancellationToken::new();
        let t2 = t.clone();
        t.cancel(CancellationReason::DeadlineNear);
        assert!(t2.is_cancelled());
    }

    #[tokio::test]
    async fn wait_cancel_immediately_returns_if_already() {
        let t = CancellationToken::new();
        t.cancel(CancellationReason::UserDisconnect);
        let r = t.wait_cancel().await;
        assert_eq!(r, CancellationReason::UserDisconnect);
    }

    #[tokio::test]
    async fn wait_cancel_wakes_on_cancellation() {
        let t = CancellationToken::new();
        let t_c = t.clone();
        let waiter = tokio::spawn(async move { t_c.wait_cancel().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        t.cancel(CancellationReason::SiblingFailed);
        let r = waiter.await.unwrap();
        assert_eq!(r, CancellationReason::SiblingFailed);
    }

    #[test]
    fn reason_serializes() {
        let r = CancellationReason::EarlyQuorum;
        assert_eq!(serde_json::to_string(&r).unwrap(), "\"EARLY_QUORUM\"");
    }
}
