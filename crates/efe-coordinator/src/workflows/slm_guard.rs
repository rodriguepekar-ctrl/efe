// SPDX-License-Identifier: UNLICENSED
//! **R3.S2** — Helpers de guard SLM circuit breaker pour les workflows.
//!
//! Module factorisé qui encapsule le pattern §2.2 de l'architecture R3.S2 :
//!
//! 1. **Avant invoke** : `slm_guard_check()` → `Skip`/`Proceed` selon état
//!    du circuit. Si `Skip` (circuit Open), trace `FailureMode::SlmDown` +
//!    `failed_components` dans `ChewingMetrics`.
//! 2. **Après invoke OK** : `slm_guard_record_success()` → reset compteurs
//!    du circuit (retour en Closed).
//! 3. **Après invoke Err** : `slm_guard_record_failure()` → record_failure
//!    sur le circuit, et si la transition `Closed → Open` se produit, émet
//!    `StreamEvent::SlmCircuitOpened` sur le bus.
//!
//! Le module est `pub(crate)` — usage interne aux workflows uniquement.
//!
//! ## Justification module séparé
//!
//! Les 3 helpers sont identiques dans `run_mission_a` (3 sites) et
//! `run_mission_b` (3 sites). Au lieu de dupliquer ~150 LOC × 2 workflows,
//! on factorise dans ce module : ~120 LOC total.

use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use efe_nano_app_coordinator::{SlmCircuitBreaker, SlmCircuitState};

use crate::events::{EventBus, StreamEvent};
use crate::failure::{combine_failure_modes, FailureMode};
use crate::metrics::ChewingMetrics;

/// **R3.S2** — Résultat du `slm_guard_check`. Indique au caller s'il
/// peut appeler `*.invoke()` ou s'il doit skip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlmGuardOutcome {
    /// Le circuit est `Open` → le caller **doit** skipper l'invoke et
    /// continuer la mission en mode dégradé. `FailureMode::SlmDown` et
    /// `failed_components` ont déjà été tracés dans `ChewingMetrics`.
    Skip,
    /// Le circuit est `Closed` ou `HalfOpen` (ou non configuré) →
    /// le caller **peut** invoker. Après l'appel, appeler
    /// `slm_guard_record_success/failure` selon le résultat.
    Proceed,
}

/// **R3.S2** — Vérifie l'état du circuit AVANT `invoke()`. Si circuit
/// `Open`, trace `FailureMode::SlmDown` + `failed_components` dans
/// `chewing_metrics` et retourne `Skip`.
///
/// Si `circuit == None` (pas configuré), retourne toujours `Proceed`
/// (mode S8 backward-compat : workflow fonctionne sans circuit).
///
/// ## Garanties
/// - Pas d'effet de bord sur le circuit lui-même (juste read).
/// - Idempotent par mission : si appelé 2× pour le même robot, push 2× sur
///   `failed_components` — c'est l'appelant qui doit ne l'invoquer qu'1×
///   par robot par mission (responsabilité workflow).
/// - Composition `FailureMode` via `combine_failure_modes` → safe avec
///   d'autres failure modes pré-existants dans la même mission.
#[must_use]
pub(crate) fn slm_guard_check(
    circuit: Option<&SlmCircuitBreaker>,
    robot_name: &str,
    chewing_metrics: &mut ChewingMetrics,
) -> SlmGuardOutcome {
    let Some(c) = circuit else {
        // Pas de circuit configuré → comportement S8 (proceed).
        return SlmGuardOutcome::Proceed;
    };
    if !c.is_open() {
        // Closed ou HalfOpen → invoke autorisé.
        return SlmGuardOutcome::Proceed;
    }
    // Circuit Open → trace + skip.
    chewing_metrics.failed_components.push(robot_name.to_string());
    let prev_mode = std::mem::take(&mut chewing_metrics.failure_mode);
    chewing_metrics.failure_mode = combine_failure_modes(
        prev_mode,
        FailureMode::SlmDown(robot_name.to_string()),
    );
    chewing_metrics.degradations_count = chewing_metrics.failure_mode.count_degradations();
    SlmGuardOutcome::Skip
}

/// **R3.S2** — Enregistre un succès SLM sur le circuit (reset compteur).
///
/// No-op si `circuit == None` (pas configuré).
///
/// Appelé après un `*Client::invoke()` retournant `Ok`. Si le circuit
/// était `HalfOpen` (probe), il repasse `Closed`.
pub(crate) fn slm_guard_record_success(circuit: Option<&SlmCircuitBreaker>) {
    if let Some(c) = circuit {
        c.record_success();
    }
}

/// **R3.S2** — Enregistre un échec SLM sur le circuit.
///
/// 1. Trace `FailureMode::SlmDown(robot)` + `failed_components` push
///    dans `chewing_metrics` (TOUJOURS, même sans circuit).
/// 2. Si `circuit` configuré, `record_failure()` → potentiellement
///    transition `Closed → Open`.
/// 3. Si la transition `Closed → Open` vient de se produire (= ce call a
///    causé l'ouverture FRAÎCHE), publie `StreamEvent::SlmCircuitOpened`
///    sur le bus 1× exactement.
///
/// Retourne `true` si la transition `Closed → Open` vient de se produire,
/// `false` sinon (déjà Open, HalfOpen → Open, pas encore ouverture, ou
/// pas de circuit).
///
/// ## Sémantique d'émission (architecture §2.4 + §9 R2)
///
/// L'event n'est émis QUE sur transition **`Closed → Open`** (ouverture
/// fraîche). La transition **`HalfOpen → Open`** (probe qui échoue) N'ÉMET
/// PAS — le circuit avait déjà signalé son ouverture lors du `Closed →
/// Open` initial. Un nouveau cycle complet (`… → Closed → Open`) ré-émet.
///
/// ## Race condition tolérée
///
/// Si 2 missions concurrentes appellent simultanément alors que
/// `failures = threshold - 1`, les 2 peuvent observer `state_pre == Closed`
/// et `now_open == true` → 2 events émis. C'est rare et le consumer dédup
/// via `(robot, timestamp)`. Voir R3 §9 R3.
pub(crate) async fn slm_guard_record_failure(
    circuit: Option<&SlmCircuitBreaker>,
    robot_name: &str,
    chewing_metrics: &mut ChewingMetrics,
    bus: &Arc<dyn EventBus>,
    swarm_id: Uuid,
) -> bool {
    // 1. R3.S2 §2.6 — Backward-compat strict S8 : si circuit None, no-op
    //    complet (pas de trace failure_mode/failed_components → workflow
    //    se comporte exactement comme S8 sans le moindre changement
    //    observable côté API).
    let Some(c) = circuit else {
        return false;
    };

    // 2. Circuit configuré : trace failure dans chewing_metrics.
    chewing_metrics.failed_components.push(robot_name.to_string());
    let prev_mode = std::mem::take(&mut chewing_metrics.failure_mode);
    chewing_metrics.failure_mode = combine_failure_modes(
        prev_mode,
        FailureMode::SlmDown(robot_name.to_string()),
    );
    chewing_metrics.degradations_count = chewing_metrics.failure_mode.count_degradations();

    // 3. R3.S2 §2.4 — Capture l'état AVANT record_failure via `state()`
    //    (qui distingue Closed / Open / HalfOpen, contrairement à
    //    `is_open()` qui confond Closed et HalfOpen). On émet UNIQUEMENT
    //    sur transition fraîche `Closed → Open` : HalfOpen → Open (probe
    //    fail) ne ré-émet pas (déjà signalé au Closed → Open initial).
    let state_pre = c.state();
    c.record_failure();
    let now_open = c.is_open();

    if state_pre == SlmCircuitState::Closed && now_open {
        // Transition fraîche Closed → Open : émet 1 event sur le bus.
        let _ = bus
            .publish(StreamEvent::SlmCircuitOpened {
                swarm_id,
                robot: robot_name.to_string(),
                failures: c.failure_count() as u32,
                ts: Utc::now(),
            })
            .await;
        true
    } else {
        false
    }
}

// =================================================================
// Tests
// =================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::events::InMemoryBus;

    fn mock_bus() -> Arc<dyn EventBus> {
        Arc::new(InMemoryBus::default())
    }

    #[tokio::test]
    async fn r3s2_slm_guard_check_proceeds_when_no_circuit() {
        let mut metrics = ChewingMetrics::default();
        let outcome = slm_guard_check(None, "analyzer", &mut metrics);
        assert_eq!(outcome, SlmGuardOutcome::Proceed);
        assert!(metrics.failed_components.is_empty());
        assert_eq!(metrics.failure_mode, FailureMode::None);
    }

    #[tokio::test]
    async fn r3s2_slm_guard_check_proceeds_when_circuit_closed() {
        let cb = SlmCircuitBreaker::new(3, Duration::from_secs(10));
        // Fresh = Closed.
        let mut metrics = ChewingMetrics::default();
        let outcome = slm_guard_check(Some(&cb), "analyzer", &mut metrics);
        assert_eq!(outcome, SlmGuardOutcome::Proceed);
        assert!(metrics.failed_components.is_empty());
    }

    #[tokio::test]
    async fn r3s2_slm_guard_check_skips_when_circuit_open() {
        let cb = SlmCircuitBreaker::new(1, Duration::from_secs(10));
        cb.record_failure(); // → Open (threshold=1).
        assert!(cb.is_open());

        let mut metrics = ChewingMetrics::default();
        let outcome = slm_guard_check(Some(&cb), "analyzer", &mut metrics);
        assert_eq!(outcome, SlmGuardOutcome::Skip);
        assert_eq!(metrics.failed_components, vec!["analyzer".to_string()]);
        assert_eq!(
            metrics.failure_mode,
            FailureMode::SlmDown("analyzer".to_string()),
            "circuit Open doit set FailureMode::SlmDown"
        );
        assert_eq!(metrics.degradations_count, 1);
    }

    #[tokio::test]
    async fn r3s2_slm_guard_check_composite_with_existing_failure_mode() {
        let cb_a = SlmCircuitBreaker::new(1, Duration::from_secs(10));
        cb_a.record_failure();
        let cb_v = SlmCircuitBreaker::new(1, Duration::from_secs(10));
        cb_v.record_failure();

        let mut metrics = ChewingMetrics::default();
        // 1er guard : analyzer Open.
        let _ = slm_guard_check(Some(&cb_a), "analyzer", &mut metrics);
        // 2ème guard : verifier Open (failure mode existant doit être composé).
        let _ = slm_guard_check(Some(&cb_v), "verifier", &mut metrics);

        assert_eq!(metrics.failed_components.len(), 2);
        // Composite([SlmDown(analyzer), SlmDown(verifier)]).
        if let FailureMode::Composite(modes) = &metrics.failure_mode {
            assert_eq!(modes.len(), 2);
            assert!(modes.contains(&FailureMode::SlmDown("analyzer".into())));
            assert!(modes.contains(&FailureMode::SlmDown("verifier".into())));
        } else {
            panic!("attendu Composite, got {:?}", metrics.failure_mode);
        }
        assert_eq!(metrics.degradations_count, 2);
    }

    #[tokio::test]
    async fn r3s2_slm_guard_record_success_resets_circuit() {
        let cb = SlmCircuitBreaker::new(3, Duration::from_secs(10));
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.failure_count(), 2);
        slm_guard_record_success(Some(&cb));
        assert_eq!(cb.failure_count(), 0, "record_success doit reset le compteur");
    }

    #[tokio::test]
    async fn r3s2_slm_guard_record_success_noop_when_no_circuit() {
        // No-op si circuit None — vérifie qu'il n'y a pas de panic.
        slm_guard_record_success(None);
    }

    #[tokio::test]
    async fn r3s2_slm_guard_record_failure_no_circuit_is_strict_noop() {
        // R3.S2 §2.6 — Backward-compat S8 strict : pas de circuit → no-op
        // complet (pas de trace failure_mode ni failed_components).
        let mut metrics = ChewingMetrics::default();
        let bus = mock_bus();
        let swarm = Uuid::new_v4();
        let opened = slm_guard_record_failure(
            None,
            "analyzer",
            &mut metrics,
            &bus,
            swarm,
        )
        .await;
        assert!(!opened, "pas de circuit → pas d'opening");
        // R3.S2 strict S8 compat : RIEN tracé sans circuit.
        assert!(
            metrics.failed_components.is_empty(),
            "backward-compat S8 : pas de circuit → failed_components vide"
        );
        assert_eq!(
            metrics.failure_mode,
            FailureMode::None,
            "backward-compat S8 : pas de circuit → failure_mode reste None"
        );
    }

    #[tokio::test]
    async fn r3s2_slm_guard_record_failure_emits_event_on_transition() {
        let cb = SlmCircuitBreaker::new(2, Duration::from_secs(10));
        let bus = mock_bus();
        let mut metrics = ChewingMetrics::default();
        let swarm = Uuid::new_v4();

        // 1ère failure → pas open → pas d'event.
        let opened1 = slm_guard_record_failure(
            Some(&cb),
            "analyzer",
            &mut metrics,
            &bus,
            swarm,
        )
        .await;
        assert!(!opened1, "1ère failure (1/2) ne doit pas ouvrir le circuit");
        assert!(!cb.is_open());

        // 2ème failure → Open → 1 event.
        let opened2 = slm_guard_record_failure(
            Some(&cb),
            "analyzer",
            &mut metrics,
            &bus,
            swarm,
        )
        .await;
        assert!(opened2, "2ème failure (2/2) doit ouvrir le circuit + emit event");
        assert!(cb.is_open());

        // 3ème failure → déjà open → PAS de nouvel event.
        let opened3 = slm_guard_record_failure(
            Some(&cb),
            "analyzer",
            &mut metrics,
            &bus,
            swarm,
        )
        .await;
        assert!(!opened3, "3ème failure : déjà Open, pas de nouvelle transition");

        // metrics : 3 push de "analyzer" + failure_mode SlmDown (dedup'd).
        assert_eq!(metrics.failed_components.len(), 3);
        assert_eq!(
            metrics.failure_mode,
            FailureMode::SlmDown("analyzer".to_string()),
            "3× SlmDown(analyzer) doit dédup en 1 SlmDown"
        );
    }

    /// **R3.S2 §2.4 + §9 R2 (Gap 4)** — La transition `HalfOpen → Open`
    /// (probe qui échoue) NE DOIT PAS ré-émettre d'event `SlmCircuitOpened`.
    /// Seul le `Closed → Open` initial émet. Un cycle complet de recovery
    /// (`… → Closed → Open`) ré-émet.
    #[tokio::test]
    async fn r3s2_slm_guard_halfopen_to_open_does_not_reemit() {
        // threshold=1 → 1 failure ouvre. reset_timeout court pour test rapide.
        let cb = SlmCircuitBreaker::new(1, Duration::from_millis(30));
        let bus = mock_bus();
        let mut metrics = ChewingMetrics::default();
        let swarm = Uuid::new_v4();

        // 1ère failure → Closed → Open → ÉMET (1er event).
        let opened1 =
            slm_guard_record_failure(Some(&cb), "analyzer", &mut metrics, &bus, swarm)
                .await;
        assert!(opened1, "Closed → Open initial doit émettre");
        assert!(cb.is_open());

        // Attendre reset_timeout → le circuit passe HalfOpen au prochain
        // state()/is_open().
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            cb.state(),
            SlmCircuitState::HalfOpen,
            "Après reset_timeout, circuit doit être HalfOpen"
        );

        // Probe échoue : record_failure depuis HalfOpen → Open.
        // Architecture §9 R2 : NE DOIT PAS ré-émettre (déjà signalé).
        let opened2 =
            slm_guard_record_failure(Some(&cb), "analyzer", &mut metrics, &bus, swarm)
                .await;
        assert!(
            !opened2,
            "HalfOpen → Open (probe fail) NE DOIT PAS ré-émettre d'event \
             (architecture §9 R2)"
        );
        assert!(cb.is_open(), "circuit re-Open après probe fail");
    }

    /// **R3.S2 §2.4 (Gap 4)** — Un cycle complet de recovery
    /// (`Closed → Open → HalfOpen → Closed → Open`) émet 2× au total :
    /// 1 par chaque `Closed → Open`.
    #[tokio::test]
    async fn r3s2_slm_guard_full_recovery_cycle_emits_twice() {
        let cb = SlmCircuitBreaker::new(1, Duration::from_millis(30));
        let bus = mock_bus();
        let mut metrics = ChewingMetrics::default();
        let swarm = Uuid::new_v4();

        // Cycle 1 : Closed → Open → ÉMET (1).
        let e1 =
            slm_guard_record_failure(Some(&cb), "analyzer", &mut metrics, &bus, swarm)
                .await;
        assert!(e1, "1er Closed → Open émet");

        // reset_timeout → HalfOpen, puis probe SUCCESS → Closed.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(cb.state(), SlmCircuitState::HalfOpen);
        slm_guard_record_success(Some(&cb));
        assert_eq!(
            cb.state(),
            SlmCircuitState::Closed,
            "probe success → recovery → Closed"
        );

        // Cycle 2 : nouvelle failure → Closed → Open → ÉMET (2).
        let e2 =
            slm_guard_record_failure(Some(&cb), "analyzer", &mut metrics, &bus, swarm)
                .await;
        assert!(
            e2,
            "2ème Closed → Open (après recovery complète) ré-émet (cycle complet)"
        );
    }
}
