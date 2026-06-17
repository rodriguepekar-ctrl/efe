// SPDX-License-Identifier: UNLICENSED
//! **Gouvernance du TEMPS (ADR-SWARM-018)** — le `DeadlineGuard`, **jumeau temporel** du
//! [`crate::budget::BudgetGuard`]. Le budget borne le COÛT ; le deadline borne la DURÉE.
//! Troisième plafond, orthogonal aux deux autres (coût, concurrence) — même principe :
//! **borner visiblement, sceller le partiel, dire ce qui reste reproductible**.
//!
//! ## Le deadline est COOPÉRATIF, pas un `tokio::timeout` naïf
//! `run_traced` scelle la provenance **après** la topologie. Un `tokio::timeout` qui
//! *drop* la topologie en plein vol ne détruit PAS la preuve (le `ProvenanceRecorder` est
//! partagé via le sink : tout record déjà écrit y survit), mais il tranche à un point
//! **arbitraire** (au milieu d'une vague). La sonde coopérative ([`DeadlineGuard::check`],
//! injectée dans la boucle de vagues du sizer comme `DeadlineProbe`) arrête, elle, sur une
//! **frontière de vague nette** : le partiel est un préfixe propre, re-prouvable (R0).
//!
//! ## Deux étages, un seul guard
//! - **Coopératif (I-AD2)** : [`check`](DeadlineGuard::check) refuse la prochaine vague dès
//!   la fenêtre fermée → `StopReason::Deadline`, partiel scellé sur frontière de vague.
//!   Couvre le chemin **adaptatif** (consensus / generate-verify par vagues).
//! - **Backstop DUR (I-AD3)** : [`hard_limit`](DeadlineGuard::hard_limit) = deadline +
//!   grâce ; `run_traced` enveloppe la topologie d'un `tokio::time::timeout(hard_limit)`.
//!   Si une topologie NON coopérative (K fixe, T2/T3) ou un *hang* dépasse, le timeout
//!   tranche — et `run_traced` **scelle quand même le partiel** présent dans le recorder.
//!   Universel (toutes topologies), mais point de coupe arbitraire.
//!
//! ## Re-prouvable, PAS reproductible (écho ADR-SWARM-017)
//! Le point de troncature dépend du **wall-clock** (combien de vagues ont tenu avant la
//! fenêtre). Un run deadliné est donc **re-prouvable** (R0 : on re-scelle les records
//! stockés → la racine se reproduit) mais **non reproductible** (R1 : une ré-exécution
//! sur une machine plus lente/rapide tronquera ailleurs). Le statut
//! [`crate::RunTermination::DeadlineExceeded`] le signale — exactement comme un run
//! `budget_stopped` est partiel-mais-honnête.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Marge minimale du backstop dur au-dessus de la deadline (une deadline minuscule garde
/// quand même de quoi laisser la vague en vol drainer et le scellement coopératif tomber).
const MIN_GRACE: Duration = Duration::from_millis(250);
/// Marge maximale du backstop (le backstop ne double jamais la facture temps).
const MAX_GRACE: Duration = Duration::from_secs(5);

/// Garde de deadline partagée par tout un run : tient l'instant de départ + la fenêtre
/// allouée, et arme un drapeau au premier dépassement vu par la sonde coopérative. Le
/// **jumeau** atomique du [`crate::budget::BudgetGuard`] — `Send + Sync`, partagée en `Arc`.
#[derive(Debug)]
pub struct DeadlineGuard {
    start: Instant,
    budget: Duration,
    /// La sonde coopérative a-t-elle refusé ≥ 1 vague ? (→ statut `DeadlineExceeded`).
    /// `Relaxed` suffit : un seul drapeau monotone, lu une fois après le run.
    tripped: AtomicBool,
}

impl DeadlineGuard {
    /// Démarre le chrono. La fenêtre court à partir de **maintenant** (l'appel, en tête de
    /// `run_traced`) — la résolution de topologie et le pré-vol budget précèdent et ne sont
    /// volontairement PAS comptés : on borne le temps d'**exécution** de l'essaim.
    pub fn new(budget: Duration) -> Self {
        Self { start: Instant::now(), budget, tripped: AtomicBool::new(false) }
    }

    /// La fenêtre allouée (informatif / construction du backstop).
    pub fn budget(&self) -> Duration {
        self.budget
    }
    /// Temps écoulé depuis le démarrage.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
    /// Temps restant avant la fenêtre (saturé à zéro).
    pub fn remaining(&self) -> Duration {
        self.budget.saturating_sub(self.start.elapsed())
    }
    /// La fenêtre est-elle fermée ? (lecture pure, sans effet de bord).
    pub fn expired(&self) -> bool {
        self.start.elapsed() >= self.budget
    }

    /// **Sonde coopérative (I-AD2)** — consultée par le sizer AVANT d'émettre une vague.
    /// `true` ⇒ il reste du temps (émettre) ; `false` ⇒ fenêtre fermée (arrêt propre).
    /// **Effet de bord** : arme `tripped` au premier refus. C'est ce drapeau — et non
    /// l'horloge relue a posteriori — qui qualifie le run `DeadlineExceeded` : un run qui
    /// *frôle* la deadline sans jamais voir la sonde refuser reste honnêtement `Completed`.
    /// Miroir exact de [`BudgetGuard::can_pay`](crate::budget::BudgetGuard::can_pay) +
    /// `mark_exhausted`, fusionnés (le temps n'a pas de « charge » séparée à reverter).
    pub fn check(&self) -> bool {
        if self.expired() {
            self.tripped.store(true, Ordering::Relaxed);
            false
        } else {
            true
        }
    }

    /// La sonde coopérative a-t-elle tronqué le run (≥ 1 vague refusée pour cause de
    /// temps) ? Lu une fois par `run_traced` pour décider du [`crate::RunTermination`].
    pub fn tripped(&self) -> bool {
        self.tripped.load(Ordering::Relaxed)
    }

    /// **Limite du backstop DUR (I-AD3)** = `deadline + grâce`. La grâce
    /// (`deadline / 4`, bornée à `[250 ms, 5 s]`) laisse la vague en vol drainer et le
    /// scellement coopératif tomber AVANT que `tokio::time::timeout` ne tranche net. Au
    /// régime nominal (adaptatif), la sonde coopérative a déjà arrêté bien avant cette
    /// limite ; le backstop ne se déclenche que sur le non-coopératif ou un *hang*.
    pub fn hard_limit(&self) -> Duration {
        let grace = (self.budget / 4).clamp(MIN_GRACE, MAX_GRACE);
        self.budget.saturating_add(grace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_passes_while_window_open_and_arms_on_expiry() {
        // Fenêtre déjà fermée (0 ns) : la sonde refuse et arme `tripped`.
        let g = DeadlineGuard::new(Duration::ZERO);
        assert!(!g.check(), "fenêtre nulle : sonde refuse d'emblée");
        assert!(g.tripped(), "le refus arme le drapeau (→ DeadlineExceeded)");
        assert!(g.expired());
        assert_eq!(g.remaining(), Duration::ZERO, "restant saturé à zéro");
    }

    #[test]
    fn fresh_guard_is_open_and_not_tripped() {
        // Fenêtre large : la sonde laisse passer et n'arme rien (run honnêtement Completed).
        let g = DeadlineGuard::new(Duration::from_secs(3600));
        assert!(g.check(), "fenêtre ouverte : sonde laisse émettre");
        assert!(!g.tripped(), "aucun refus ⇒ pas DeadlineExceeded (honnête)");
        assert!(!g.expired());
        assert!(g.remaining() > Duration::from_secs(3500));
    }

    #[test]
    fn hard_limit_brackets_the_deadline_with_bounded_grace() {
        // Petite deadline : grâce plancher 250 ms (jamais 0).
        let small = DeadlineGuard::new(Duration::from_millis(100));
        assert_eq!(small.hard_limit(), Duration::from_millis(100) + MIN_GRACE);
        // Grande deadline : grâce plafonnée à 5 s (le backstop ne double pas le temps).
        let big = DeadlineGuard::new(Duration::from_secs(60));
        assert_eq!(big.hard_limit(), Duration::from_secs(60) + MAX_GRACE);
        // Régime intermédiaire : grâce = deadline / 4.
        let mid = DeadlineGuard::new(Duration::from_secs(8));
        assert_eq!(mid.hard_limit(), Duration::from_secs(8) + Duration::from_secs(2));
    }
}
