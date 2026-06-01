// SPDX-License-Identifier: UNLICENSED
//! Sprint **D19 (P1-Foundation)** — `BudgetTracker` middleware.
//!
//! Cf. `docs/EFE_P1_SURFACE_ARCHITECTURE.md` § 7.2 (deadline
//! propagation) et § 2.1 (canonical 12-stage budget).
//!
//! ## Mission
//!
//! Propager une **deadline absolue** (`Instant`) à travers les 12
//! étages du hot-path P1, allouer une quote-part de budget à chaque
//! étage, et **avorter au plus tôt** (cancel + DeadlineExceeded) si
//! le budget restant est inférieur à ce qu'on s'apprête à dépenser.
//!
//! L'objectif n'est PAS de garantir que chaque étage reste sous son
//! budget canonique (= `Stage::budget()`, cf. p1_instruments.rs).
//! L'objectif est de garantir que **le roundtrip total** reste sous
//! le plafond contractuel — typiquement 250 ms (= 200 ms p99 SLO
//! C-1 + 25 % marge anti-tail).
//!
//! ## Modèle conceptuel
//!
//! ```text
//!  t = 0                                        t = 250 ms (deadline)
//!  ┌────────────────────────────────────────────────────────┐
//!  │ ETAGE 1: 1.5 ms  │ ÉTAGE 2: 0.5 ms │ ÉTAGE 3: 3 ms │ … │
//!  └────────────────────────────────────────────────────────┘
//!     ↑              ↑
//!  Tracker.new()   Tracker.checkpoint(Stage::GrpcDecode) :
//!                  - mesure le wall-clock écoulé de l'étage précédent
//!                  - enregistre observe_stage()
//!                  - vérifie qu'on a > Stage::GrpcDecode.budget() de
//!                    budget restant ; sinon → DeadlineExceeded
//! ```
//!
//! ## Invariants pinés
//!
//! 1. `BudgetTracker` est **non-cloneable** : il représente une
//!    requête unique. Le partage entre tâches concurrentes est
//!    explicitement INTERDIT.
//! 2. La méthode `checkpoint(Stage)` doit être appelée à la fin
//!    de chaque étage **dans l'ordre canonique** (déclaré par
//!    `Stage::all()`).
//! 3. La méthode `finish()` consomme le tracker et émet le
//!    `efe_p1_budget_remaining_seconds` histogram.
//! 4. Si une étape excède son budget canonique, le compteur
//!    `efe_p1_budget_overrun_total{stage}` est incrémenté MAIS
//!    le tracker ne retourne pas une erreur — la marge anti-tail
//!    peut absorber. C'est seulement la **deadline globale** qui
//!    déclenche `DeadlineExceeded`.

use std::time::{Duration, Instant};

use efe_telemetry::metrics::p1_instruments::{
    note_budget_overrun, observe_budget_remaining, observe_inference, observe_stage, Stage,
};

// ====================================================================
// Erreur publique
// ====================================================================

/// Erreur retournée quand la deadline a été dépassée.
///
/// Cette erreur est convertie en `tonic::Status::deadline_exceeded`
/// par le glue `?` de l'extracteur `BudgetTracker` côté handler.
#[derive(Debug, Clone, thiserror::Error)]
pub enum BudgetError {
    /// La deadline globale est dépassée avant de pouvoir commencer
    /// l'étage en question.
    #[error("budget deadline exceeded before stage `{stage}` (remaining = {remaining_ms} ms)")]
    DeadlineExceeded {
        /// Stage canonique (lowercase, e.g. "fusion").
        stage: &'static str,
        /// Temps restant à l'instant de la mesure (peut être négatif
        /// si la déadline est déjà passée ; on l'expose en ms signed).
        remaining_ms: f64,
    },
    /// Le tracker a reçu un `checkpoint` pour un étage déjà visité.
    /// Indique un bug d'ordre des appels côté handler.
    #[error("stage `{stage}` already checkpointed (double-call detected)")]
    DoubleCheckpoint {
        /// Stage incriminé.
        stage: &'static str,
    },
}

// ====================================================================
// Configuration
// ====================================================================

/// Configuration du tracker.
#[derive(Debug, Clone)]
pub struct BudgetConfig {
    /// Le plafond strict du roundtrip — au-delà, la requête est
    /// cancel.
    ///
    /// Défaut : **250 ms** (= 200 ms SLO C-1 + 25 % marge).
    pub hard_deadline: Duration,
    /// Mode label utilisé pour les métriques (cardinalité contrôlée :
    /// `mode1` / `mode2` / `mode3`).
    pub mode: &'static str,
    /// Si `true`, le tracker incrémente `efe_p1_budget_overrun_total`
    /// dès qu'un étage dépasse `Stage::budget()`. Si `false`, le
    /// compteur n'est pas touché (utile pour les tests qui veulent
    /// piner des durées sans polluer le compteur).
    pub emit_overruns: bool,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            hard_deadline: Duration::from_millis(250),
            mode: "mode1",
            emit_overruns: true,
        }
    }
}

impl BudgetConfig {
    /// Mode-1 GPU configuration (par défaut).
    #[must_use]
    pub const fn mode1() -> Self {
        Self {
            hard_deadline: Duration::from_millis(250),
            mode: "mode1",
            emit_overruns: true,
        }
    }

    /// Mode-2 (LLM-routed anchors) — deadline plus large pour le LLM call.
    /// Cf. C-3 contract : 1500 ms p99.
    #[must_use]
    pub const fn mode2() -> Self {
        Self {
            hard_deadline: Duration::from_millis(1800),
            mode: "mode2",
            emit_overruns: false, // budgets P1 non-pertinents en Mode-2
        }
    }

    /// Mode-3 (multi-turn). Cf. C-4 contract : 3× Mode-1.
    #[must_use]
    pub const fn mode3() -> Self {
        Self {
            hard_deadline: Duration::from_millis(700),
            mode: "mode3",
            emit_overruns: false,
        }
    }
}

// ====================================================================
// Le tracker
// ====================================================================

/// Garde-fou de la deadline par requête.
///
/// Non-clonable, non-`Send` à dessein (chaque requête a son propre
/// tracker, qui vit sur la même tâche Tokio). Si une tâche enfant
/// doit reporter du progrès, elle passe le `BudgetSnapshot` (cf.
/// `snapshot()`), pas le tracker lui-même.
#[derive(Debug)]
pub struct BudgetTracker {
    cfg: BudgetConfig,
    /// Instant du début de la requête (= `recv` côté serveur).
    started_at: Instant,
    /// Deadline absolue.
    deadline: Instant,
    /// Instant du dernier `checkpoint` réussi.
    last_checkpoint: Instant,
    /// Bitset des étages déjà checkpointed (12 bits = 1 u16).
    visited: u16,
}

impl BudgetTracker {
    /// Construit un nouveau tracker. La deadline démarre **maintenant**.
    #[must_use]
    pub fn new(cfg: BudgetConfig) -> Self {
        let now = Instant::now();
        Self {
            deadline: now + cfg.hard_deadline,
            cfg,
            started_at: now,
            last_checkpoint: now,
            visited: 0,
        }
    }

    /// Construit un tracker avec une deadline donnée explicitement
    /// (utile quand le client gRPC pousse un `grpc-timeout` header
    /// plus court que `hard_deadline`).
    #[must_use]
    pub fn with_deadline(cfg: BudgetConfig, deadline: Instant) -> Self {
        let now = Instant::now();
        // Si la deadline du client est encore plus courte que la
        // hard_deadline du serveur, on respecte la plus courte. Si
        // c'est l'inverse, on garde la hard_deadline serveur.
        let effective = std::cmp::min(deadline, now + cfg.hard_deadline);
        Self {
            cfg,
            started_at: now,
            deadline: effective,
            last_checkpoint: now,
            visited: 0,
        }
    }

    /// Temps restant avant la deadline. Peut être négatif si on a
    /// déjà dépassé — exposé comme `Duration::ZERO` ici (le bit signed
    /// est conservé par `remaining_signed_ms`).
    #[must_use]
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// Temps restant **signé** (peut être négatif). Utile pour les
    /// logs de diagnostic post-mortem.
    #[must_use]
    pub fn remaining_signed_ms(&self) -> f64 {
        let now = Instant::now();
        if now <= self.deadline {
            (self.deadline - now).as_secs_f64() * 1000.0
        } else {
            -((now - self.deadline).as_secs_f64() * 1000.0)
        }
    }

    /// Temps écoulé depuis le début de la requête (sans rapport
    /// avec la deadline ; utile pour les traces).
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Allocation pré-flight pour l'étage `stage` : retourne le
    /// `Duration` qu'il **peut** consommer au maximum. C'est la
    /// **plus petite** de :
    /// - le budget canonique de l'étage (`Stage::budget()`),
    /// - le temps restant avant la deadline.
    ///
    /// Si le restant est strictement inférieur au budget canonique,
    /// la fonction retourne ce restant ; c'est au caller de décider
    /// si c'est suffisant ou s'il préfère échouer immédiatement
    /// (en pratique, utiliser `tokio::time::timeout(allocate_for_stage(s), …)`).
    ///
    /// **Returns `Err(DeadlineExceeded)`** si le temps restant est
    /// nul ou négatif.
    pub fn allocate_for_stage(&self, stage: Stage) -> Result<Duration, BudgetError> {
        let remaining = self.remaining();
        if remaining.is_zero() {
            return Err(BudgetError::DeadlineExceeded {
                stage: stage.as_label(),
                remaining_ms: self.remaining_signed_ms(),
            });
        }
        Ok(remaining.min(stage.budget()))
    }

    /// Marque la fin de l'étage `stage`. Observe la durée mesurée
    /// dans Prometheus et — si overshoot — incrémente le compteur
    /// d'overrun.
    ///
    /// **Retourne `Err(DoubleCheckpoint)`** si l'étage a déjà été
    /// visité dans la vie du tracker (= bug d'ordre des appels).
    pub fn checkpoint(&mut self, stage: Stage) -> Result<Duration, BudgetError> {
        let bit = 1_u16 << stage_index(stage);
        if self.visited & bit != 0 {
            return Err(BudgetError::DoubleCheckpoint {
                stage: stage.as_label(),
            });
        }
        self.visited |= bit;
        let now = Instant::now();
        let duration = now.duration_since(self.last_checkpoint);
        self.last_checkpoint = now;

        observe_stage(stage, self.cfg.mode, duration);
        if self.cfg.emit_overruns && duration > stage.budget() {
            note_budget_overrun(self.cfg.mode, stage);
        }
        Ok(duration)
    }

    /// Variante de `checkpoint` pour l'étage 5 (inférence) qui
    /// transmet la taille du batch effectivement passé au GPU.
    pub fn checkpoint_inference(&mut self, batch_size: usize) -> Result<Duration, BudgetError> {
        let bit = 1_u16 << stage_index(Stage::EmbedInference);
        if self.visited & bit != 0 {
            return Err(BudgetError::DoubleCheckpoint {
                stage: Stage::EmbedInference.as_label(),
            });
        }
        self.visited |= bit;
        let now = Instant::now();
        let duration = now.duration_since(self.last_checkpoint);
        self.last_checkpoint = now;
        observe_inference(self.cfg.mode, batch_size, duration);
        if self.cfg.emit_overruns && duration > Stage::EmbedInference.budget() {
            note_budget_overrun(self.cfg.mode, Stage::EmbedInference);
        }
        Ok(duration)
    }

    /// Snapshot copiable et `Send` du tracker. Utilisé quand on doit
    /// passer le restant à une tâche enfant (ex. spawn d'un GC
    /// d'audit chain).
    #[must_use]
    pub fn snapshot(&self) -> BudgetSnapshot {
        BudgetSnapshot {
            mode: self.cfg.mode,
            deadline: self.deadline,
            started_at: self.started_at,
        }
    }

    /// Termine le tracking. Observe le budget restant et clôt la
    /// requête. **Doit être appelé exactement une fois** à la fin
    /// de chaque requête (réussite OU échec — appeler en `Drop`-like
    /// pattern via `tokio::scopeguard` ou un `defer`).
    pub fn finish(self) {
        let remaining = self.remaining();
        observe_budget_remaining(self.cfg.mode, remaining);
    }

    /// Inspection seulement : nombre d'étages déjà visités.
    #[must_use]
    pub fn visited_count(&self) -> u32 {
        // `u16::count_ones` retourne déjà `u32` ; pas de conversion
        // intermédiaire (audit fix D19-P1.A7-2).
        self.visited.count_ones()
    }

    /// Mode label.
    #[must_use]
    pub fn mode(&self) -> &'static str {
        self.cfg.mode
    }
}

/// Index canonique d'un `Stage` dans la séquence du hot-path.
///
/// `panic`-free : `Stage::all()` est exhaustif.
const fn stage_index(s: Stage) -> u32 {
    match s {
        Stage::GrpcRecv => 0,
        Stage::GrpcDecode => 1,
        Stage::Interceptors => 2,
        Stage::Tokenize => 3,
        Stage::EmbedInference => 4,
        Stage::Genesis => 5,
        Stage::Fusion => 6,
        Stage::Consensus => 7,
        Stage::DissolutionAlignment => 8,
        Stage::AuditChainAppend => 9,
        Stage::GrpcEncode => 10,
        Stage::GrpcSend => 11,
    }
}

// ====================================================================
// Snapshot
// ====================================================================

/// Snapshot copiable d'un tracker, sans état mutable. Permet à une
/// tâche enfant de lire la deadline sans posséder le tracker.
#[derive(Debug, Clone, Copy)]
pub struct BudgetSnapshot {
    /// Mode label canonique.
    pub mode: &'static str,
    /// Deadline absolue.
    pub deadline: Instant,
    /// Début de la requête.
    pub started_at: Instant,
}

impl BudgetSnapshot {
    /// Temps restant.
    #[must_use]
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
}

// ====================================================================
// Glue tonic — conversion `BudgetError → Status`
// ====================================================================
//
// **Audit fix D19-P1.A2** — l'impl était initialement gated par
// `#[cfg(feature = "tonic-status")]`, mais ce feature n'est pas
// déclaré dans `Cargo.toml` ; le code était donc **dead** et clippy
// le signalait (`unexpected cfg condition value`). Puisque
// `efe-server` dépend déjà de `tonic` de manière inconditionnelle,
// l'impl reste active dans toutes les builds.

impl From<BudgetError> for tonic::Status {
    fn from(err: BudgetError) -> Self {
        match err {
            BudgetError::DeadlineExceeded {
                stage,
                remaining_ms,
            } => tonic::Status::deadline_exceeded(format!(
                "P1 budget deadline exceeded at stage `{stage}` (remaining = {remaining_ms:.2} ms)"
            )),
            BudgetError::DoubleCheckpoint { stage } => {
                tonic::Status::internal(format!("P1 tracker double-checkpoint at stage `{stage}`"))
            }
        }
    }
}

// ====================================================================
// Tests unitaires inline (rapides ; les tests d'intégration vivent
// dans tests/p1_budget_tracker.rs).
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn new_default_has_hard_deadline_250ms() {
        let t = BudgetTracker::new(BudgetConfig::default());
        let r = t.remaining();
        assert!(r <= Duration::from_millis(250));
        assert!(r > Duration::from_millis(248), "should be ~250ms initially");
    }

    #[test]
    fn mode1_mode2_mode3_have_distinct_deadlines() {
        let t1 = BudgetTracker::new(BudgetConfig::mode1());
        let t2 = BudgetTracker::new(BudgetConfig::mode2());
        let t3 = BudgetTracker::new(BudgetConfig::mode3());
        assert_eq!(t1.mode(), "mode1");
        assert_eq!(t2.mode(), "mode2");
        assert_eq!(t3.mode(), "mode3");
        // Le `remaining` initial doit refléter la `hard_deadline`.
        assert!(t1.remaining() <= Duration::from_millis(250));
        assert!(t2.remaining() <= Duration::from_millis(1800));
        assert!(t3.remaining() <= Duration::from_millis(700));
    }

    #[test]
    fn allocate_for_stage_returns_stage_budget_when_plenty_of_room() {
        let t = BudgetTracker::new(BudgetConfig::default());
        let alloc = t.allocate_for_stage(Stage::Fusion).expect("not zero");
        assert_eq!(alloc, Stage::Fusion.budget());
    }

    #[test]
    fn allocate_for_stage_clamps_to_remaining_when_short() {
        let cfg = BudgetConfig {
            hard_deadline: Duration::from_millis(50), // moins que Fusion budget
            ..BudgetConfig::default()
        };
        let t = BudgetTracker::new(cfg);
        let alloc = t.allocate_for_stage(Stage::Fusion).expect("not zero");
        // Fusion budget = 100 ms, deadline = 50 ms → alloc clampée à ≤ 50 ms
        assert!(alloc <= Duration::from_millis(50));
        assert!(alloc > Duration::from_millis(48), "should be ~50ms");
    }

    #[test]
    fn allocate_for_stage_after_deadline_fails() {
        let cfg = BudgetConfig {
            hard_deadline: Duration::from_millis(1),
            ..BudgetConfig::default()
        };
        let t = BudgetTracker::new(cfg);
        sleep(Duration::from_millis(5));
        let err = t.allocate_for_stage(Stage::Tokenize).unwrap_err();
        assert!(matches!(err, BudgetError::DeadlineExceeded { .. }));
    }

    #[test]
    fn checkpoint_observes_and_advances() {
        let mut t = BudgetTracker::new(BudgetConfig {
            emit_overruns: false,
            ..BudgetConfig::default()
        });
        sleep(Duration::from_micros(500));
        let d = t.checkpoint(Stage::GrpcDecode).expect("ok");
        assert!(d >= Duration::from_micros(500));
        assert!(d < Duration::from_millis(50)); // sanity bound
        assert_eq!(t.visited_count(), 1);
    }

    #[test]
    fn double_checkpoint_is_rejected() {
        let mut t = BudgetTracker::new(BudgetConfig {
            emit_overruns: false,
            ..BudgetConfig::default()
        });
        t.checkpoint(Stage::Tokenize).expect("first ok");
        let err = t.checkpoint(Stage::Tokenize).unwrap_err();
        assert!(matches!(err, BudgetError::DoubleCheckpoint { .. }));
    }

    #[test]
    fn snapshot_is_consistent() {
        let t = BudgetTracker::new(BudgetConfig::default());
        let s = t.snapshot();
        assert_eq!(s.mode, t.mode());
        // Le restant du snapshot doit être très proche de celui du
        // tracker (à un Instant::now() près).
        let d_diff = (s.remaining().as_secs_f64() - t.remaining().as_secs_f64()).abs();
        assert!(d_diff < 0.001);
    }

    #[test]
    fn finish_observes_remaining() {
        let t = BudgetTracker::new(BudgetConfig::default());
        sleep(Duration::from_millis(2));
        // Pas de panic = ok.
        t.finish();
    }

    #[test]
    fn with_deadline_respects_min_of_client_and_server() {
        let cfg = BudgetConfig {
            hard_deadline: Duration::from_millis(250),
            ..BudgetConfig::default()
        };
        let client_deadline = Instant::now() + Duration::from_millis(80);
        let t = BudgetTracker::with_deadline(cfg, client_deadline);
        // La deadline effective doit être ≤ 80 ms (client plus serré).
        assert!(t.remaining() <= Duration::from_millis(80));
        assert!(t.remaining() > Duration::from_millis(78));
    }

    #[test]
    fn with_deadline_client_longer_than_server_uses_server() {
        let cfg = BudgetConfig {
            hard_deadline: Duration::from_millis(250),
            ..BudgetConfig::default()
        };
        let client_deadline = Instant::now() + Duration::from_secs(60);
        let t = BudgetTracker::with_deadline(cfg, client_deadline);
        // La deadline effective doit être ≤ 250 ms (serveur plus serré).
        assert!(t.remaining() <= Duration::from_millis(250));
        assert!(t.remaining() > Duration::from_millis(248));
    }

    #[test]
    fn stage_index_is_dense_and_distinct() {
        let mut indices: Vec<u32> = Stage::all().iter().map(|s| stage_index(*s)).collect();
        indices.sort();
        assert_eq!(indices, (0..12).collect::<Vec<u32>>());
    }
}
