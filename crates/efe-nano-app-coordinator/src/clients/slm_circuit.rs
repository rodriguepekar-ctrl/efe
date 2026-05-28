// SPDX-License-Identifier: UNLICENSED
//! **Sprint S8 R2** — Circuit breaker per-robot SLM (analyzer/compressor/verifier).
//!
//! Distinct du [`CircuitBreaker`](super) du crate `efe-coordinator` qui est
//! per-LLM-provider. Celui-ci protège les workflows contre les
//! `llama-server` crashs ou timeouts répétés sur les nano-robots Phase 1.
//!
//! ## Sémantique
//!
//! - **Closed (initial)** : tous les `invoke()` passent normalement.
//! - **Open** : `invoke()` court-circuité — le workflow doit skip le robot
//!   (= comportement S2 naked sans pré-mâchage).
//! - **HalfOpen** : après `reset_timeout`, 1 `invoke()` est autorisé. Si
//!   succès → Closed. Si échec → Open + timeout préservé (pas d'exp backoff
//!   contrairement au CircuitBreaker LLM pour rester simple).
//!
//! ## Seuils choisis
//!
//! - `threshold = 3` (vs 5 pour LLM provider) : on tolère moins d'échecs
//!   car un SLM local est censé être très disponible. 3 timeouts = problème
//!   réel (crash llama-server, OOM…).
//! - `reset_timeout = 10s` (vs 60s pour LLM) : redémarrage local rapide.
//!
//! ## Usage typique
//!
//! ```ignore
//! use efe_nano_app_coordinator::clients::slm_circuit::SlmCircuitBreaker;
//! use std::time::Duration;
//!
//! let circuit = SlmCircuitBreaker::new(3, Duration::from_secs(10));
//! // Avant chaque invoke SLM :
//! if circuit.is_open() {
//!     // → skip le robot, emit SLM_CIRCUIT_OPENED si pas déjà émis
//! } else {
//!     match analyzer.invoke(input, ctx).await {
//!         Ok(out) => { circuit.record_success(); /* ... */ }
//!         Err(e)  => { circuit.record_failure(); /* skip robot pour cette mission */ }
//!     }
//! }
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// État du circuit SLM (identique à `CircuitBreaker` LLM).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlmCircuitState {
    /// Tout va bien — appels autorisés.
    Closed,
    /// Trop d'échecs SLM récents — appels bloqués jusqu'à `reset_timeout`.
    Open,
    /// Période de probe — 1 appel autorisé pour tester.
    HalfOpen,
}

/// Circuit breaker per-robot SLM (analyzer/compressor/verifier).
#[derive(Clone)]
pub struct SlmCircuitBreaker {
    inner: Arc<Inner>,
}

struct Inner {
    /// Échecs consécutifs récents.
    failures: AtomicU64,
    /// 0 = Closed, 1 = Open, 2 = HalfOpen.
    state: AtomicU8,
    /// Dernière transition vers Open.
    last_open_at: Mutex<Option<Instant>>,
    /// Seuil avant ouverture (recommandé 3 pour SLM).
    threshold: u32,
    /// Durée avant transition Open → HalfOpen.
    reset_timeout: Duration,
}

impl SlmCircuitBreaker {
    /// Construit un circuit breaker SLM fermé.
    ///
    /// - `threshold` : nombre d'échecs consécutifs avant ouverture (3 recommandé).
    /// - `reset_timeout` : durée avant tentative half-open (10 s recommandé).
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
    /// Auto-transition Open → HalfOpen si `reset_timeout` écoulé.
    #[must_use]
    pub fn state(&self) -> SlmCircuitState {
        match self.inner.state.load(Ordering::SeqCst) {
            0 => SlmCircuitState::Closed,
            1 => {
                let guard = self.inner.last_open_at.lock().expect("mutex poisoned");
                if let Some(open_at) = *guard {
                    if open_at.elapsed() >= self.inner.reset_timeout {
                        self.inner.state.store(2, Ordering::SeqCst);
                        return SlmCircuitState::HalfOpen;
                    }
                }
                SlmCircuitState::Open
            }
            _ => SlmCircuitState::HalfOpen,
        }
    }

    /// `true` si un `invoke()` doit être court-circuité (Open).
    #[must_use]
    pub fn is_open(&self) -> bool {
        matches!(self.state(), SlmCircuitState::Open)
    }

    /// Enregistre un succès — reset compteurs, retour en Closed.
    pub fn record_success(&self) {
        self.inner.failures.store(0, Ordering::SeqCst);
        self.inner.state.store(0, Ordering::SeqCst);
    }

    /// Enregistre un échec. Si seuil atteint, ouvre le circuit.
    pub fn record_failure(&self) {
        let f = self.inner.failures.fetch_add(1, Ordering::SeqCst) + 1;
        if f >= self.inner.threshold as u64 {
            self.inner.state.store(1, Ordering::SeqCst);
            let mut guard = self.inner.last_open_at.lock().expect("mutex poisoned");
            *guard = Some(Instant::now());
        }
    }

    /// Échecs comptés depuis la dernière success (snapshot).
    #[must_use]
    pub fn failure_count(&self) -> u64 {
        self.inner.failures.load(Ordering::SeqCst)
    }

    /// Seuil configuré (pour audit + bus events).
    #[must_use]
    pub fn threshold(&self) -> u32 {
        self.inner.threshold
    }
}

impl std::fmt::Debug for SlmCircuitBreaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlmCircuitBreaker")
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
        let cb = SlmCircuitBreaker::new(3, Duration::from_millis(100));
        assert_eq!(cb.state(), SlmCircuitState::Closed);
        assert!(!cb.is_open());
        assert_eq!(cb.failure_count(), 0);
        assert_eq!(cb.threshold(), 3);
    }

    #[test]
    fn opens_after_threshold_3_failures() {
        let cb = SlmCircuitBreaker::new(3, Duration::from_millis(100));
        cb.record_failure();
        assert_eq!(cb.state(), SlmCircuitState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), SlmCircuitState::Closed);
        cb.record_failure();
        // 3 failures = threshold → Open.
        assert_eq!(cb.state(), SlmCircuitState::Open);
        assert!(cb.is_open());
    }

    #[test]
    fn success_resets_to_closed() {
        let cb = SlmCircuitBreaker::new(3, Duration::from_millis(100));
        cb.record_failure();
        cb.record_failure();
        cb.record_success();
        assert_eq!(cb.state(), SlmCircuitState::Closed);
        assert_eq!(cb.failure_count(), 0);
    }

    #[test]
    fn half_open_after_reset_timeout() {
        let cb = SlmCircuitBreaker::new(2, Duration::from_millis(30));
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), SlmCircuitState::Open);
        std::thread::sleep(Duration::from_millis(50));
        // Après timeout → HalfOpen.
        assert_eq!(cb.state(), SlmCircuitState::HalfOpen);
        assert!(!cb.is_open());
    }

    #[test]
    fn clones_share_state() {
        let cb = SlmCircuitBreaker::new(2, Duration::from_millis(50));
        let cb2 = cb.clone();
        cb.record_failure();
        cb.record_failure();
        assert!(cb2.is_open(), "clone partage l'état atomique");
    }

    #[test]
    fn debug_format_includes_state() {
        let cb = SlmCircuitBreaker::new(3, Duration::from_millis(100));
        let s = format!("{cb:?}");
        assert!(s.contains("Closed"));
        assert!(s.contains("threshold: 3"));
    }
}
