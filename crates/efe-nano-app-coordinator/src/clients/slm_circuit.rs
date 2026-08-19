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
//! - **HalfOpen** : après `reset_timeout`, période de sonde. Deux portes,
//!   aux garanties DIFFÉRENTES :
//!   - [`SlmCircuitBreaker::try_call`] (recommandée, Fil B.2) : **1 sonde à
//!     la fois** — le premier appelant acquiert le slot, les autres sont
//!     court-circuités jusqu'au verdict (`record_success` → Closed,
//!     `record_failure` → Open + fenêtre fraîche). Si la sonde ne rend
//!     jamais de verdict (caller crashé), le slot est repris après
//!     `reset_timeout` (borne anti-blocage).
//!   - [`SlmCircuitBreaker::is_open`] (peek historique S8, sans effet de
//!     bord) : TOUS les appelants d'une fenêtre HalfOpen passent
//!     (multi-sonde). C'est la porte des workflows mission A/B via
//!     `slm_guard` — honnêteté : elle ne protège PAS d'un troupeau
//!     concurrent pendant HalfOpen ; seul `try_call` le fait.
//!
//!   Échec de sonde → Open + timeout préservé (pas d'exp backoff
//!   contrairement au CircuitBreaker LLM, pour rester simple).
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
//! // Avant chaque invoke SLM (porte 1-sonde, recommandée) :
//! if !circuit.try_call() {
//!     // → skip le robot (Open, ou sonde HalfOpen déjà en vol)
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
    /// Période de probe. Via [`SlmCircuitBreaker::try_call`] : 1 sonde à la
    /// fois. Via [`SlmCircuitBreaker::is_open`] (peek) : tous les appelants
    /// passent.
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
    /// 0 = Closed, 1 = Open, 2 = HalfOpen (slot de sonde libre),
    /// 3 = HalfOpen (sonde en vol — porte `try_call` prise).
    state: AtomicU8,
    /// Horodatages protégés : toute TRANSITION d'état se fait sous ce mutex
    /// (le load rapide `state == 0` reste lock-free sur le chemin chaud).
    times: Mutex<Times>,
    /// Seuil avant ouverture (recommandé 3 pour SLM).
    threshold: u32,
    /// Durée avant transition Open → HalfOpen (réutilisée comme borne de
    /// reprise d'une sonde sans verdict).
    reset_timeout: Duration,
}

#[derive(Default)]
struct Times {
    /// Dernière transition vers Open.
    last_open_at: Option<Instant>,
    /// Départ de la sonde en vol (état 3) — pour la reprise anti-blocage.
    probe_started_at: Option<Instant>,
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
                times: Mutex::new(Times::default()),
                threshold,
                reset_timeout,
            }),
        }
    }

    /// Snapshot atomique de l'état courant.
    /// Auto-transition Open → HalfOpen (slot libre) si `reset_timeout` écoulé.
    #[must_use]
    pub fn state(&self) -> SlmCircuitState {
        match self.inner.state.load(Ordering::SeqCst) {
            0 => SlmCircuitState::Closed,
            1 => {
                let guard = self.inner.times.lock().expect("mutex poisoned");
                // Re-lecture sous le mutex : `record_failure` écrit le
                // timestamp AVANT de publier state=1 (même mutex), donc un
                // Open observé ici a toujours un timestamp frais.
                if self.inner.state.load(Ordering::SeqCst) != 1 {
                    drop(guard);
                    return self.state();
                }
                if let Some(open_at) = guard.last_open_at {
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
    ///
    /// **Peek sans effet de bord** (porte historique S8, multi-sonde) : en
    /// HalfOpen, TOUS les appelants voient `false` et peuvent invoker. Pour
    /// la porte 1-sonde-à-la-fois, utiliser [`Self::try_call`].
    #[must_use]
    pub fn is_open(&self) -> bool {
        matches!(self.state(), SlmCircuitState::Open)
    }

    /// **Porte à acquisition** (Fil B.2) — `true` si CE caller peut invoker.
    ///
    /// - `Closed` → `true` (tout le monde passe, lock-free).
    /// - `Open` non expiré → `false`.
    /// - `Open` expiré / `HalfOpen` : **une sonde à la fois** — le premier
    ///   appelant acquiert le slot (`true`), les suivants sont
    ///   court-circuités (`false`) jusqu'au verdict de la sonde
    ///   ([`Self::record_success`] → Closed, [`Self::record_failure`] →
    ///   Open + fenêtre fraîche). Si la sonde ne rend jamais de verdict
    ///   (caller crashé, erreur côté client sans record), le slot est
    ///   repris après `reset_timeout` — borne anti-blocage.
    ///
    /// Contrairement à [`Self::is_open`], cette porte protège un
    /// `llama-server` malade d'un troupeau concurrent pendant HalfOpen :
    /// un seul hop paie la sonde (jusqu'au timeout client), les autres
    /// dégradent sans appel réseau.
    #[must_use]
    pub fn try_call(&self) -> bool {
        loop {
            match self.inner.state.load(Ordering::SeqCst) {
                0 => return true,
                1 => {
                    let mut t = self.inner.times.lock().expect("mutex poisoned");
                    if self.inner.state.load(Ordering::SeqCst) != 1 {
                        continue; // transition concurrente, re-boucle
                    }
                    let expired = t
                        .last_open_at
                        .is_none_or(|at| at.elapsed() >= self.inner.reset_timeout);
                    if expired {
                        // Open expiré → CE caller devient LA sonde (1→3).
                        t.probe_started_at = Some(Instant::now());
                        self.inner.state.store(3, Ordering::SeqCst);
                        return true;
                    }
                    return false;
                }
                2 => {
                    let mut t = self.inner.times.lock().expect("mutex poisoned");
                    if self
                        .inner
                        .state
                        .compare_exchange(2, 3, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                    {
                        t.probe_started_at = Some(Instant::now());
                        return true;
                    }
                    continue;
                }
                _ => {
                    let mut t = self.inner.times.lock().expect("mutex poisoned");
                    if self.inner.state.load(Ordering::SeqCst) != 3 {
                        continue;
                    }
                    let stale = t
                        .probe_started_at
                        .is_none_or(|at| at.elapsed() >= self.inner.reset_timeout);
                    if stale {
                        // Sonde sans verdict depuis reset_timeout → reprise.
                        t.probe_started_at = Some(Instant::now());
                        return true;
                    }
                    return false;
                }
            }
        }
    }

    /// Enregistre un succès — reset compteurs, retour en Closed (et
    /// libération du slot de sonde le cas échéant).
    pub fn record_success(&self) {
        self.inner.failures.store(0, Ordering::SeqCst);
        let mut t = self.inner.times.lock().expect("mutex poisoned");
        t.probe_started_at = None;
        t.last_open_at = None;
        self.inner.state.store(0, Ordering::SeqCst);
    }

    /// Enregistre un échec. Si seuil atteint, ouvre le circuit avec une
    /// fenêtre FRAÎCHE.
    ///
    /// Le timestamp est écrit sous le mutex AVANT la publication de
    /// `state = Open` : un observateur concurrent qui voit Open lit
    /// toujours la fenêtre fraîche (jamais l'ancienne, expirée — sinon il
    /// re-basculerait la toute nouvelle ouverture en HalfOpen).
    pub fn record_failure(&self) {
        let f = self.inner.failures.fetch_add(1, Ordering::SeqCst) + 1;
        if f >= self.inner.threshold as u64 {
            let mut t = self.inner.times.lock().expect("mutex poisoned");
            t.last_open_at = Some(Instant::now());
            t.probe_started_at = None;
            self.inner.state.store(1, Ordering::SeqCst);
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

    // ─── Fil B.2 : porte à acquisition `try_call` ────────────────────

    #[test]
    fn try_call_passes_closed_blocks_open() {
        let cb = SlmCircuitBreaker::new(2, Duration::from_secs(60));
        assert!(cb.try_call(), "Closed : tout le monde passe");
        cb.record_failure();
        cb.record_failure();
        assert!(!cb.try_call(), "Open non expiré : court-circuité");
        assert!(!cb.try_call(), "toujours court-circuité");
    }

    #[test]
    fn half_open_grants_exactly_one_probe() {
        let cb = SlmCircuitBreaker::new(2, Duration::from_millis(30));
        cb.record_failure();
        cb.record_failure();
        std::thread::sleep(Duration::from_millis(50));
        // Fenêtre expirée : le PREMIER try_call acquiert LA sonde…
        assert!(cb.try_call(), "premier appelant = la sonde");
        // …les suivants sont court-circuités (pas de troupeau), même si
        // l'état public reste HalfOpen.
        assert!(!cb.try_call(), "sonde en vol ⟹ les autres dégradent");
        assert!(!cb.try_call());
        assert_eq!(cb.state(), SlmCircuitState::HalfOpen);
        // `is_open()` (peek S8, multi-sonde) continue de laisser passer —
        // contrat historique conservé pour slm_guard.
        assert!(!cb.is_open(), "peek historique : HalfOpen n'est pas Open");
        // Verdict succès : refermé, tout le monde repasse.
        cb.record_success();
        assert_eq!(cb.state(), SlmCircuitState::Closed);
        assert!(cb.try_call());
    }

    #[test]
    fn probe_slot_reclaimed_after_reset_timeout_without_verdict() {
        let cb = SlmCircuitBreaker::new(1, Duration::from_millis(30));
        cb.record_failure();
        std::thread::sleep(Duration::from_millis(50));
        assert!(cb.try_call(), "sonde acquise");
        assert!(!cb.try_call(), "slot pris");
        // La sonde ne rend jamais de verdict (caller crashé / ClientSide) :
        // après `reset_timeout`, le slot est repris — pas de blocage éternel.
        std::thread::sleep(Duration::from_millis(50));
        assert!(cb.try_call(), "slot repris après la borne anti-blocage");
    }

    #[test]
    fn probe_failure_reopens_with_fresh_window() {
        let cb = SlmCircuitBreaker::new(1, Duration::from_millis(40));
        cb.record_failure();
        std::thread::sleep(Duration::from_millis(60));
        assert!(cb.try_call(), "sonde acquise après expiration");
        // Verdict : échec. Sans le REFRESH du timestamp (écrit sous mutex
        // AVANT la publication de state=Open), l'ancienne fenêtre (60 ms >
        // 40 ms) ferait rebasculer aussitôt en HalfOpen.
        cb.record_failure();
        assert_eq!(cb.state(), SlmCircuitState::Open, "fenêtre FRAÎCHE");
        assert!(!cb.try_call(), "Open frais : court-circuité");
    }
}
