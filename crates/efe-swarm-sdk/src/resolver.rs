// SPDX-License-Identifier: UNLICENSED
//! `Resolver` — résout un nom d'agent en `Arc<dyn Trait>` **selon le mode** :
//! - **flotte** (Mode A, keyless, déterministe) si pas de backend ;
//! - **agents LLM** (Mode C, self-host) si un backend est présent (le nom = instruction).
//!
//! Chaque agent résolu est **enveloppé** (budget + provenance). Le **vérificateur** (T1)
//! reste toujours déterministe (a-posteriori, ADR-002) — depuis la flotte.

use std::sync::Arc;

use efe_nano_sdk::AgentId;
use efe_swarm_topology::{
    builtin_arguers, builtin_refiner, AgentFleet, BudgetProbe, DeadlineProbe, Decomposer,
    Generator, Judge, Merger, PatternSelector, Refiner, Solver, Stage, StageRefiner, Verifier,
};

use crate::agents::{
    meter_decomposer, meter_gen, meter_judge, meter_merger, meter_refiner, meter_solver,
    meter_stage, Meter, StageSolver,
};
use crate::backend::LocalBackend;
use crate::deadline::DeadlineGuard;
use crate::llm::{llm_decomposer, llm_gen, llm_judge, llm_merger, llm_refiner, llm_solver, llm_stage};
use crate::result::SwarmError;

pub(crate) struct Resolver<'a> {
    pub fleet: &'a AgentFleet,
    /// `Some` ⇒ self-host : on construit des agents LLM (nom = instruction). Backend local
    /// keyless (Ollama ou OpenAI-compatible local), enveloppé d'un circuit-breaker.
    pub backend: Option<Arc<dyn LocalBackend>>,
    pub meter: Meter,
    /// **ADR-014 (CP-1)** : largeur d'exécution du run, résolue par `run_traced`
    /// (cœurs Mode A / backend déclaré Mode C / env / spec) — descendue dans chaque
    /// topologie par `TopologySpec::resolve`. Jamais une sémantique (I-CP1).
    pub width: usize,
    /// **ADR-018 (I-AD2)** : garde de deadline du run (`Some` ⇒ une fenêtre temporelle est
    /// posée). `run_traced` la construit ; `deadline_probe()` l'enveloppe en sonde
    /// coopérative descendue dans les topologies adaptatives. `None` ⇒ aucun deadline
    /// (chemin historique inchangé — zéro surcoût, aucune sonde branchée).
    pub deadline: Option<Arc<DeadlineGuard>>,
    /// **ADR-019 (MEM-2, I-MEM4)** : warm-start de `k_min` suggéré par la mémoire (prior de
    /// taille), déjà clampé aux bornes du run par `run_traced`. `Some(k)` ⇒ le sizer adaptatif
    /// démarre près du K historique ; `None` ⇒ départ standard (aucune mémoire / pas de donnée).
    /// La mémoire AMORCE, ne gouverne jamais : `warm_start_k_min` re-clampe par sécurité.
    pub memory_k_min: Option<usize>,
    /// **ADR-019 (MEM-2.b)** : exemplaires few-shot recall-és (aperçus de gagnants passés de la
    /// famille), à injecter dans le prompt des générateurs **Mode C** (`llm_gen`). En Mode A
    /// (flotte keyless), il n'y a pas de prompt → ignorés (extraction sans effet, honnête). Vide
    /// ⇒ prompt historique inchangé (I-MEM8). Bornés en amont (≤ `memory::FEWSHOT_N`).
    pub memory_exemplars: Vec<String>,
    /// **ADR-019 (read-back A)** : signaux bruts du prior de pattern (depuis l'instantané pinné),
    /// pour BIAISER le sélecteur adaptatif. `None` ⇒ aucun biais (sélecteur = base, I-PAT2).
    pub memory_pattern_bias: Option<efe_swarm_memory::PatternBias>,
    /// **ADR-019 (read-back B)** : la cellule de réputation (depuis l'instantané pinné), pour
    /// calculer les poids de routage par-variante. `None` ⇒ agrégation uniforme (I-REP2).
    pub memory_reputation: Option<efe_swarm_memory::Reputation>,
    /// **ADR-019 (read-back B, R-b/R-c)** : la **version** de l'instantané pinné (= nombre de folds
    /// repliés dans la famille) — la **clé de rotation** de l'ordonnancement de profils. Pinnée avec
    /// la réputation depuis le MÊME `MemorySnapshot` (cohérence atomique). `None` ⇒ pas de mémoire ⇒
    /// `total = 0` ⇒ ordonnancement identité (dormance, I-REP10).
    pub memory_version: Option<u64>,
}

fn aid(s: &str) -> AgentId {
    AgentId::new(s).expect("agent id SDK valide")
}

impl Resolver<'_> {
    /// **I-MEM4** : le `k_min` effectif après warm-start mémoire — l'amorce, **bornée** à
    /// `[k_min, k_max]`. Sans mémoire (ou hors données), renvoie le `k_min` d'origine inchangé.
    /// Défense en profondeur : même si l'amorce était hors bornes, le `clamp` la ramène — la
    /// mémoire ne peut JAMAIS pousser `k_min` au-delà de `k_max` ni sous le plancher du run.
    pub fn warm_start_k_min(&self, k_min: usize, k_max: usize) -> usize {
        match self.memory_k_min {
            Some(w) => w.clamp(k_min, k_max),
            None => k_min,
        }
    }

    /// **ADR-019 (read-back A, temps ② du patron §1)** : le sélecteur de pattern **biaisé** par la
    /// mémoire. `θ_eff = clamp(θ_base + κ·conf·(escal − ρ), θ_lo, θ_hi)` — un taux d'escalade
    /// historique élevé (famille piégeuse) **monte** le seuil (exiger plus d'accord avant le Vote) ;
    /// bas, il le **descend** (économiser les juges). `use_condorcet` suit la dispersion dominante
    /// **si** la confiance dépasse un plancher. **Sans prior ⇒ base inchangée** (I-PAT2). Tout est
    /// déterministe (fonction du prior PINNÉ) → la décision est re-prouvable (I-PAT3). Le **veto des
    /// candidats** (le sélecteur reste piloté par l'accord réel) garde la borne dure (I-PAT1).
    pub fn biased_selector(&self, base: PatternSelector) -> PatternSelector {
        apply_pattern_bias(base, self.memory_pattern_bias.as_ref())
    }

    /// **ADR-019 (read-back B, R-a)** : les **poids de routage** par-variante pour `0..k_max`
    /// (f64 dans la bande `[W_LO,W_HI]/1000`), depuis la cellule de réputation **pinnée**. L'indice
    /// `i` ↔ la variante `generator_{i:03}`. `None` ⇒ aucun routage (agrégation uniforme, I-REP2).
    /// Déterministe → re-prouvable (I-REP4). Le SDK pré-calcule pour `0..k_max` ; le consensus
    /// tranche `[0..k_used]` au vol.
    pub fn reputation_weights_for(&self, k_max: usize) -> Option<Vec<f64>> {
        let rep = self.memory_reputation.as_ref()?;
        let ids: Vec<String> = (0..k_max).map(|i| format!("generator_{i:03}")).collect();
        Some(rep.routing_weights(&ids).into_iter().map(|w| w as f64 / 1000.0).collect())
    }

    /// **ADR-019 (read-back B, R-b/R-c)** : l'**ordonnancement de profils** du sizer adaptatif —
    /// une permutation de `[0, k_max)` où `schedule[p]` = le profil à exécuter à la position `p`
    /// (priorisation réputationnelle R-b + rotation d'exploration déterministe R-c, keyée sur la
    /// **version** pinnée). Jumeau de [`reputation_weights_for`](Self::reputation_weights_for) :
    /// même réputation pinnée, même indexation par profil (`generator_{i:03}`). `None` ⇒ pas de
    /// mémoire ⇒ le consensus prend son chemin **identité** (byte-identique, I-REP10). Déterministe
    /// (fonction de l'instantané pinné + bornes spec) ⇒ re-prouvable (I-AK3′, I-REP8). Le SDK
    /// pré-calcule pour `0..k_max` ; le sizer lit `schedule[0..k_used]` au vol.
    pub fn reputation_schedule_for(&self, k_min: usize, k_max: usize) -> Option<Vec<usize>> {
        let rep = self.memory_reputation.as_ref()?;
        Some(rep.profile_schedule(self.memory_version.unwrap_or(0), k_min, k_max))
    }
}

/// **ADR-019 (read-back A, temps ② — PUR + testable)** : le transfert du prior de pattern vers un
/// sélecteur biaisé. `θ_eff = clamp(θ_base + κ·conf·(escal − ρ), θ_lo, θ_hi)`, `conf = min(1,
/// total/N)`. Sans prior (`None`) ⇒ base inchangée (I-PAT2). Bornes dures (I-PAT1). Déterministe.
pub fn apply_pattern_bias(base: PatternSelector, bias: Option<&efe_swarm_memory::PatternBias>) -> PatternSelector {
    let Some(bias) = bias.filter(|b| b.total > 0) else {
        // pas de mémoire OU instantané genèse (total==0) → base RIGOUREUSEMENT inchangée (I-PAT2).
        // Sans ce garde, le `.clamp([θ_lo,θ_hi])` ci-dessous réécrirait une base hors-bande même à
        // confiance nulle (p. ex. 0,50 → 0,70) : une violation latente de l'inertie sans donnée.
        return base;
    };
    let conf = (bias.total.min(PATTERN_N_CONF) as f64) / PATTERN_N_CONF as f64; // min(1, total/N)
    let escal = bias.escalation_rate_milli as f64 / 1000.0;
    let theta = (base.vote_threshold + PATTERN_KAPPA * conf * (escal - PATTERN_RHO)).clamp(THETA_LO, THETA_HI);
    let use_condorcet = match (&bias.dominant_dispersion, conf >= PATTERN_CONF_FLOOR) {
        (Some(d), true) => d == "condorcet",
        _ => base.use_condorcet, // pas de marge nette / confiance insuffisante → base
    };
    PatternSelector::new(theta, use_condorcet)
}

// ── read-back A : constantes du biais de pattern (le clamp + l'échelle de confiance) ──
/// Nombre de runs pour atteindre la pleine confiance (`conf = min(1, total/N)`).
const PATTERN_N_CONF: u64 = 30;
/// Gain du biais sur le seuil de vote (amplitude maximale du déplacement de θ).
const PATTERN_KAPPA: f64 = 0.2;
/// Taux d'escalade de référence (au-dessus ⇒ plus exigeant ; en dessous ⇒ plus permissif).
const PATTERN_RHO: f64 = 0.3;
/// Plancher de confiance pour adopter la méthode de dispersion dominante de la mémoire.
const PATTERN_CONF_FLOOR: f64 = 0.5;
/// Bornes du seuil de vote effectif — la bande sans dégénérescence (jamais toujours-Vote/toujours-juges).
const THETA_LO: f64 = 0.70;
const THETA_HI: f64 = 0.97;

impl Resolver<'_> {
    /// **I-AK4 (ADR-014)** : la sonde de budget coopérative du sizer — enveloppe le
    /// `BudgetGuard` du run (« n appels de plus, payables entièrement ? »). La
    /// topologie reste découplée du modèle de prix. Un refus MARQUE l'épuisement :
    /// le chemin coopératif n'a aucun `charge()` refusé pour le faire — sans ça, un
    /// run tronqué par le budget se déclarerait propre (`budget_stopped: false`).
    pub fn budget_probe(&self) -> BudgetProbe {
        let guard = self.meter.guard.clone();
        Arc::new(move |calls: usize| {
            let ok = guard.can_pay(calls);
            if !ok {
                guard.mark_exhausted();
            }
            ok
        })
    }

    /// **I-AD2 (ADR-018)** : la sonde de deadline coopérative — le **jumeau temporel** de
    /// [`budget_probe`](Self::budget_probe). Enveloppe le [`DeadlineGuard`] du run
    /// (« la fenêtre temporelle reste-t-elle ouverte ? ») ; un refus MARQUE la troncature
    /// (via `check`), tout comme le refus budgétaire marque l'épuisement. `None` si aucun
    /// deadline n'est posé sur le run (la topologie ne branche alors aucune sonde — chemin
    /// historique strictement inchangé). La topologie reste découplée de l'horloge.
    pub fn deadline_probe(&self) -> Option<DeadlineProbe> {
        let guard = self.deadline.clone()?;
        Some(Arc::new(move || guard.check()))
    }

    pub fn generator(&self, name: &str, id: &str) -> Result<Arc<dyn Generator>, SwarmError> {
        let g = match &self.backend {
            // ADR-019 (MEM-2.b) : en Mode C, le générateur reçoit les exemplaires few-shot
            // recall-és (cf. `memory_exemplars`). Vides ⇒ prompt historique (I-MEM8).
            Some(b) => llm_gen(b.clone(), name.to_string(), self.memory_exemplars.clone()),
            None => self.fleet.generator(name).ok_or_else(|| SwarmError::UnknownAgent { role: "generator", name: name.to_string() })?,
        };
        Ok(meter_gen(g, self.meter.clone(), aid(id)))
    }

    /// Vérificateur : **toujours déterministe** (flotte), même en self-host.
    pub fn verifier(&self, name: &str) -> Result<Arc<dyn Verifier>, SwarmError> {
        self.fleet.verifier(name).ok_or_else(|| SwarmError::UnknownAgent { role: "verifier", name: name.to_string() })
    }

    pub fn stage(&self, name: &str, id: &str) -> Result<Arc<dyn Stage>, SwarmError> {
        let s = match &self.backend {
            Some(b) => llm_stage(b.clone(), name.to_string()),
            None => self.fleet.stage(name).ok_or_else(|| SwarmError::UnknownAgent { role: "stage", name: name.to_string() })?,
        };
        Ok(meter_stage(s, self.meter.clone(), aid(id)))
    }

    pub fn decomposer(&self, name: &str, id: &str) -> Result<Arc<dyn Decomposer>, SwarmError> {
        let d = match &self.backend {
            Some(b) => llm_decomposer(b.clone(), name.to_string()),
            None => self.fleet.decomposer(name).ok_or_else(|| SwarmError::UnknownAgent { role: "decomposer", name: name.to_string() })?,
        };
        Ok(meter_decomposer(d, self.meter.clone(), aid(id)))
    }

    pub fn solver(&self, name: &str, id: &str) -> Result<Arc<dyn Solver>, SwarmError> {
        // pas de rôle solver dans la flotte → on adapte une STAGE (Mode A) ; en self-host,
        // un solveur LLM direct.
        let s: Arc<dyn Solver> = match &self.backend {
            Some(b) => llm_solver(b.clone(), name.to_string()),
            None => Arc::new(StageSolver(self.fleet.stage(name).ok_or_else(|| SwarmError::UnknownAgent { role: "solver(stage)", name: name.to_string() })?)),
        };
        Ok(meter_solver(s, self.meter.clone(), aid(id)))
    }

    pub fn merger(&self, name: &str, id: &str) -> Result<Arc<dyn Merger>, SwarmError> {
        let m = match &self.backend {
            Some(b) => llm_merger(b.clone(), name.to_string()),
            None => self.fleet.merger(name).ok_or_else(|| SwarmError::UnknownAgent { role: "merger", name: name.to_string() })?,
        };
        Ok(meter_merger(m, self.meter.clone(), aid(id)))
    }


    /// **T4 Gossip** : résout un refiner. Mode C = instruction LLM ; Mode A = refiner
    /// intégré réservé (`mean_pull`, `median_pull`, `hold`, `majority_adopt`), sinon
    /// adaptation d'une `Stage` de flotte (l'étage reçoit `{task, own, peers}`).
    pub fn refiner(&self, name: &str, id: &str) -> Result<Arc<dyn Refiner>, SwarmError> {
        let r: Arc<dyn Refiner> = match &self.backend {
            Some(b) => llm_refiner(b.clone(), name.to_string()),
            None => match builtin_refiner(name) {
                Some(r) => r,
                None => Arc::new(StageRefiner(self.fleet.stage(name).ok_or_else(|| {
                    SwarmError::UnknownAgent { role: "refiner", name: name.to_string() }
                })?)),
            },
        };
        Ok(meter_refiner(r, self.meter.clone(), aid(id)))
    }

    /// **Debate** : résout la paire (PRO, CONTRA). Mode C = instructions LLM ;
    /// Mode A = arguers déterministes intégrés (les noms servent d'étiquettes).
    pub fn arguers(&self, pro: &str, contra: &str) -> (Arc<dyn Generator>, Arc<dyn Generator>) {
        let (p, c): (Arc<dyn Generator>, Arc<dyn Generator>) = match &self.backend {
            // Débat : PAS de few-shot mémoire — l'exemplaire recall-é est un gagnant de
            // CONSENSUS (réponse agrégée), de forme étrangère à un argument pro/contra. On ne
            // l'injecte donc que dans `generator()`. (ADR-019, périmètre honnête.)
            Some(b) => (llm_gen(b.clone(), pro.to_string(), Vec::new()), llm_gen(b.clone(), contra.to_string(), Vec::new())),
            None => builtin_arguers(false),
        };
        (meter_gen(p, self.meter.clone(), aid("pro")), meter_gen(c, self.meter.clone(), aid("contra")))
    }

    pub fn judge(&self, name: &str, id: &str) -> Result<Arc<dyn Judge>, SwarmError> {
        let j = match &self.backend {
            Some(b) => llm_judge(b.clone(), name.to_string()),
            None => self.fleet.judge(name).ok_or_else(|| SwarmError::UnknownAgent { role: "judge", name: name.to_string() })?,
        };
        Ok(meter_judge(j, self.meter.clone(), aid(id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_swarm_memory::PatternBias;

    #[test]
    fn pattern_bias_transfer_is_bounded_and_inert_without_data() {
        let base = PatternSelector::new(0.9, false);

        // sans prior → base inchangée (I-PAT2).
        let s = apply_pattern_bias(base.clone(), None);
        assert_eq!((s.vote_threshold, s.use_condorcet), (0.9, false));

        // famille PIÉGEUSE (escalade haute, pleine confiance) → seuil MONTE, clampé à θ_hi (I-PAT1).
        let trap = PatternBias { escalation_rate_milli: 900, dominant_dispersion: None, total: 100 };
        let s = apply_pattern_bias(base.clone(), Some(&trap));
        assert!(s.vote_threshold > 0.9, "piégeuse → plus exigeant : {}", s.vote_threshold);
        assert!(s.vote_threshold <= THETA_HI, "clampé à θ_hi (jamais toujours-juges)");

        // famille FIABLE (escalade basse) → seuil DESCEND, clampé à θ_lo.
        let safe = PatternBias { escalation_rate_milli: 0, dominant_dispersion: None, total: 100 };
        let s = apply_pattern_bias(base.clone(), Some(&safe));
        assert!(s.vote_threshold < 0.9, "fiable → trust agreement : {}", s.vote_threshold);
        assert!(s.vote_threshold >= THETA_LO, "clampé à θ_lo (jamais toujours-Vote)");

        // PEU de confiance (total faible) → biais minime (échelle de confiance).
        let weak = PatternBias { escalation_rate_milli: 900, dominant_dispersion: None, total: 1 };
        let s = apply_pattern_bias(base.clone(), Some(&weak));
        assert!((s.vote_threshold - 0.9).abs() < 0.05, "faible confiance → quasi-base : {}", s.vote_threshold);

        // total==0 (instantané GENÈSE) → base RIGOUREUSEMENT inchangée, MÊME hors-bande (I-PAT2) —
        // le clamp ne doit pas mordre à confiance nulle (le défaut que la revue a trouvé).
        let oob = PatternSelector::new(0.50, true);
        let genesis = PatternBias { escalation_rate_milli: 900, dominant_dispersion: Some("condorcet".into()), total: 0 };
        let s = apply_pattern_bias(oob, Some(&genesis));
        assert_eq!((s.vote_threshold, s.use_condorcet), (0.50, true), "genèse (total==0) → base inchangée, hors-bande comprise");

        // dispersion dominante condorcet (pleine confiance) → use_condorcet.
        let cond = PatternBias { escalation_rate_milli: 500, dominant_dispersion: Some("condorcet".into()), total: 100 };
        assert!(apply_pattern_bias(base, Some(&cond)).use_condorcet, "condorcet dominant → use_condorcet");
    }
}
