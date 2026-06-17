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
    Generator, Judge, Merger, Refiner, Solver, Stage, StageRefiner, Verifier,
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
}

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
            Some(b) => llm_gen(b.clone(), name.to_string()),
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
            Some(b) => (llm_gen(b.clone(), pro.to_string()), llm_gen(b.clone(), contra.to_string())),
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
