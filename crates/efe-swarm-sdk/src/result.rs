// SPDX-License-Identifier: UNLICENSED
//! Résultat & erreurs du SDK.

use efe_swarm_topology::QualitySignals;
use serde::Serialize;
use serde_json::Value;

use crate::budget::BudgetSnapshot;

/// Comptabilité d'un run (issue de `TopologyOutput`).
#[derive(Debug, Clone, Serialize)]
pub struct Accounting {
    pub agents_used: usize,
    /// Unités d'appel générateur (comptabilité topologie).
    pub cost_units: f64,
    pub elapsed_ms: u64,
}

/// Résumé de provenance d'un run (le différenciateur produit).
#[derive(Debug, Clone, Serialize)]
pub struct ProvenanceSummary {
    pub enabled: bool,
    pub records: usize,
    /// Fraction d'enregistrements ancrés à une cellule durable.
    pub traced_pct: f64,
    /// `verify_swarm` a-t-il re-prouvé le run (chaîne + contenu + DAG) ?
    pub verified: bool,
    /// `provenance_root` — un seul hash qui prouve tout le run, re-vérifiable par un tiers.
    pub root: Option<String>,
}

impl ProvenanceSummary {
    pub(crate) fn disabled() -> Self {
        Self { enabled: false, records: 0, traced_pct: 1.0, verified: false, root: None }
    }
}

/// Le résultat **complet** d'un `swarm.run` : sortie + coût + budget + preuve + qualité.
#[derive(Debug, Clone, Serialize)]
pub struct SwarmRun {
    /// La réponse de l'essaim.
    pub output: Value,
    /// Nom de la topologie effectivement exécutée (après éventuelle dégradation).
    pub topology: String,
    pub accounting: Accounting,
    /// Dépense exacte (cents) + ventilation — **ne dépasse jamais le plafond** (I13).
    pub budget: BudgetSnapshot,
    /// Preuve re-vérifiable du run.
    pub provenance: ProvenanceSummary,
    /// Signaux de qualité (candidats, vérifiés, sous-tâches).
    #[serde(skip)]
    pub quality: QualitySignals,
    /// Le budget a-t-il été épuisé en cours de run (politique `Stop`) → résultat partiel.
    pub budget_stopped: bool,
}

impl SwarmRun {
    /// Le run est-il intègre ET dans le budget (pas d'arrêt prématuré) ?
    pub fn is_clean(&self) -> bool {
        (!self.provenance.enabled || self.provenance.verified) && !self.budget_stopped
    }
}

/// Erreurs du SDK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwarmError {
    /// Un agent nommé est introuvable dans la flotte.
    UnknownAgent { role: &'static str, name: String },
    /// Spécification vide (ex. pipeline sans étage).
    EmptySpec(&'static str),
    /// Politique `Error` : le coût estimé dépasse le plafond AVANT exécution.
    BudgetTooLow { needed_cents: u32, max_cents: u32 },
    /// Politique `Error` : le budget a été dépassé en cours d'exécution.
    BudgetExceeded { spent_cents: u32, max_cents: u32 },
    /// Échec de dérivation de la clé de provenance.
    KeyError(String),
    /// `ClientMode::Network` nécessite la **passerelle serveur** (clés server-side) — non
    /// exécutable en in-process. Utilisez `Keyless` ou `SelfHost`.
    NetworkRequiresGateway,
}

impl std::fmt::Display for SwarmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SwarmError::UnknownAgent { role, name } => write!(f, "agent {role} « {name} » introuvable dans la flotte"),
            SwarmError::EmptySpec(s) => write!(f, "spécification vide : {s}"),
            SwarmError::BudgetTooLow { needed_cents, max_cents } => {
                write!(f, "budget trop bas : ~{needed_cents}¢ requis > plafond {max_cents}¢")
            }
            SwarmError::BudgetExceeded { spent_cents, max_cents } => {
                write!(f, "budget dépassé : {spent_cents}¢ dépensés / plafond {max_cents}¢")
            }
            SwarmError::KeyError(e) => write!(f, "clé de provenance : {e}"),
            SwarmError::NetworkRequiresGateway => {
                write!(f, "ClientMode::Network nécessite la passerelle serveur (clés server-side) ; utilisez Keyless ou SelfHost en in-process")
            }
        }
    }
}

impl std::error::Error for SwarmError {}
