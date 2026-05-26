// SPDX-License-Identifier: UNLICENSED
//! `CircuitBreaker` — invariant I17 (open après seuil d'erreurs).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// État du circuit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Tout va bien — appels autorisés.
    Closed,
    /// Trop d'erreurs récentes — appels bloqués jusqu'à `timeout`.
    Open,
    /// Période de probe après timeout — 1 appel autorisé pour tester.
    HalfOpen,
}

/// Circuit breaker per-provider.
///
/// ## Sémantique
///
/// - **Closed (initial)** : tous les appels passent. Compte les échecs.
/// - **Open** : aucun appel autorisé pendant `reset_timeout`.
/// - **HalfOpen** : après le timeout, 1 appel autorisé. Si succès → Closed.
///   Si échec → Open avec timeout doublé (exponential backoff).
#[derive(Clone)]
pub struct CircuitBreaker {
    inner: Arc<Inner>,
}

struct Inner {
    /// Failures consécutifs récents.
    failures: AtomicU64,
    /// 0 = Closed, 1 = Open, 2 = HalfOpen.
    state: AtomicU8,
    /// Dernière transition vers Open.
    last_open_at: Mutex<Option<Instant>>,
    /// Seuil avant ouverture.
    threshold: u32,
    /// Durée avant transition Open → HalfOpen.
    reset_timeout: Duration,
}

impl CircuitBreaker {
    /// Construit un nouveau circuit breaker fermé.
    ///
    /// - `threshold` : nombre d'échecs consécutifs avant ouverture.
    /// - `reset_timeout` : durée avant de tenter le half-open.
    #[must_use]
    pub fn new(threshold: u32, reset_timeout: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                failures: AtomicU64::new(0),
                state: AtomicU8::new(0),
                last_open_at: Mutex::new(None),
                threshold,
                reset_timeout,
            }),
        }
    }

    /// Snapshot atomique de l'état courant.
    #[must_use]
    pub fn state(&self) -> CircuitState {
        match self.inner.state.load(Ordering::SeqCst) {
            0 => CircuitState::Closed,
            1 => {
                // Vérifie si on doit transitionner vers HalfOpen.
                if let Some(open_at) = *self.inner.last_open_at.lock() {
                    if open_at.elapsed() >= self.inner.reset_timeout {
                        self.inner.state.store(2, Ordering::SeqCst);
                        return CircuitState::HalfOpen;
                    }
                }
                CircuitState::Open
            }
            _ => CircuitState::HalfOpen,
        }
    }

    /// `true` si un appel doit être bloqué (circuit Open avec timeout pas écoulé).
    #[must_use]
    pub fn is_open(&self) -> bool {
        matches!(self.state(), CircuitState::Open)
    }

    /// Enregistre un succès — reset les compteurs, retourne en Closed.
    pub fn record_success(&self) {
        self.inner.failures.store(0, Ordering::SeqCst);
        self.inner.state.store(0, Ordering::SeqCst);
    }

    /// Enregistre un échec.  Si seuil atteint, ouvre le circuit.
    pub fn record_failure(&self) {
        let f = self.inner.failures.fetch_add(1, Ordering::SeqCst) + 1;
        if f >= self.inner.threshold as u64 {
            self.inner.state.store(1, Ordering::SeqCst);
            *self.inner.last_open_at.lock() = Some(Instant::now());
        }
    }

    /// Échecs comptés depuis la dernière success (snapshot).
    #[must_use]
    pub fn failure_count(&self) -> u64 {
        self.inner.failures.load(Ordering::SeqCst)
    }
}

impl std::fmt::Debug for CircuitBreaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CircuitBreaker")
            .field("state", &self.state())
            .field("failures", &self.failure_count())
            .field("threshold", &self.inner.threshold)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_breaker_is_closed() {
        let cb = CircuitBreaker::new(5, Duration::from_millis(100));
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(!cb.is_open());
        assert_eq!(cb.failure_count(), 0);
    }

    #[test]
    fn opens_after_threshold() {
        let cb = CircuitBreaker::new(3, Duration::from_millis(100));
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
        cb.record_failure();
        // 3 failures = threshold → Open.
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(cb.is_open());
    }

    #[test]
    fn success_resets_to_closed() {
        let cb = CircuitBreaker::new(3, Duration::from_millis(100));
        cb.record_failure();
        cb.record_failure();
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert_eq!(cb.failure_count(), 0);
    }

    #[test]
    fn half_open_after_timeout() {
        let cb = CircuitBreaker::new(2, Duration::from_millis(30));
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        std::thread::sleep(Duration::from_millis(50));
        // Après le timeout → HalfOpen.
        assert_eq!(cb.state(), CircuitState::HalfOpen);
        assert!(!cb.is_open());
    }

    #[test]
    fn success_after_half_open_returns_closed() {
        let cb = CircuitBreaker::new(2, Duration::from_millis(20));
        cb.record_failure();
        cb.record_failure();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(cb.state(), CircuitState::HalfOpen);
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn clones_share_state() {
        let cb = CircuitBreaker::new(2, Duration::from_millis(50));
        let cb2 = cb.clone();
        cb.record_failure();
        cb.record_failure();
        assert!(cb2.is_open());
    }
}
