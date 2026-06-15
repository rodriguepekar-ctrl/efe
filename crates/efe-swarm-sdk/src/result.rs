// SPDX-License-Identifier: UNLICENSED
//! Résultat & erreurs du SDK.

use efe_swarm_provenance::SealedProvenance;
use efe_swarm_topology::QualitySignals;
use serde::Serialize;
use serde_json::Value;

use crate::budget::BudgetSnapshot;

/// Signaux de qualité **sérialisables** (miroir de `QualitySignals`, qui ne dérive pas
/// `Serialize` côté topologie).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct QualitySummary {
    pub candidates: usize,
    pub verified: usize,
    pub subtasks: usize,
}

impl From<QualitySignals> for QualitySummary {
    fn from(q: QualitySignals) -> Self {
        Self { candidates: q.candidates, verified: q.verified, subtasks: q.subtasks }
    }
}

/// Comptabilité d'un run (issue de `TopologyOutput`).
#[derive(Debug, Clone, Serialize)]
pub struct Accounting {
    pub agents_used: usize,
    /// Unités d'appel générateur (comptabilité topologie).
    pub cost_units: f64,
    pub elapsed_ms: u64,
    /// **ADR-014** : largeur d'exécution RÉSOLUE du run (cœurs / backend / spec) —
    /// l'observabilité « exec_width » de l'architecture §3.3, réalisée par-run (champ
    /// additif) plutôt qu'en jauge Prometheus (une jauge par-run n'a pas de sens ;
    /// déviation actée ADR-014). I-CP1 : informative, jamais une sémantique.
    pub width: usize,
}

/// Descripteur **forensique** du backend local d'un run self-host (alimente le pont N17 :
/// *quel moteur, quel modèle, où*). Aucune clé — l'endpoint est local. `None` en keyless.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackendInfo {
    /// Moteur local : `ollama` ou `openai_local` (llama.cpp/vLLM/TGI/LM Studio).
    pub engine: String,
    pub model: String,
    /// Endpoint local (souverain — jamais un secret).
    pub endpoint: String,
}

/// Résumé du **pont N17 par-appel** : chaque appel d'agent Mode C a produit un
/// `LlmProvenanceRecord` forensique (modèle/params/hashs), référencé par l'`AgentCallRecord`
/// via `llm_provenance_id`. `None` pour un run keyless (Mode A — aucun appel LLM).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct N17Summary {
    /// Nombre d'appels LLM forensiquement enregistrés (records N17).
    pub calls: usize,
    /// Nombre d'`AgentCallRecord` Mode C (ceux qui doivent porter un lien).
    pub mode_c_records: usize,
    /// Mode C **liés et cohérents** (N17 existe, même tenant, même swarm).
    pub consistent: usize,
    /// Mode C **sans** lien (lacunes).
    pub unlinked: usize,
    /// Liens **rompus** (pointeur pendant / incohérence d'isolation).
    pub broken: usize,
    /// Tous les Mode C sont liés et cohérents (aucune lacune, aucune rupture).
    pub all_consistent: bool,
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
    pub quality: QualitySummary,
    /// Le budget a-t-il été épuisé en cours de run (politique `Stop`) → résultat partiel.
    pub budget_stopped: bool,
    /// Backend local du run self-host (forensique, pont N17). `None` en keyless (Mode A).
    pub backend: Option<BackendInfo>,
    /// Résumé du pont N17 par-appel (records LLM forensiques + cohérence des liens). `None`
    /// en keyless (Mode A).
    pub n17: Option<N17Summary>,
}

impl SwarmRun {
    /// Le run est-il intègre ET dans le budget (pas d'arrêt prématuré) ?
    pub fn is_clean(&self) -> bool {
        (!self.provenance.enabled || self.provenance.verified) && !self.budget_stopped
    }
}

/// Résultat **tracé** d'un run : le [`SwarmRun`] (résumé sérialisable) **plus** la
/// provenance scellée ([`SealedProvenance`]) — les `AgentCallRecord` bruts (l'évidence
/// re-vérifiable par `verify_swarm`).
///
/// [`crate::Swarm::run`] (Inc.1) jette `sealed` et ne renvoie que le résumé ;
/// [`crate::Swarm::run_traced`] (Inc.2) l'**expose** pour que la passerelle HTTP
/// persiste les records et serve `GET /v1/swarm/verify/:id` (le *moat* exposé).
///
/// N'est volontairement **pas** `Serialize` : `SealedProvenance` porte les records bruts
/// (sérialisés à part par la passerelle) ; le résumé sérialisable est `self.run`.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    /// Le résumé public du run (sortie + coût + budget + preuve + qualité).
    pub run: SwarmRun,
    /// Provenance scellée (records chaînés + `provenance_root`) — `None` si la provenance
    /// est désactivée sur le `Swarm`.
    pub sealed: Option<SealedProvenance>,
    /// **Pont N17 par-appel** : les `LlmProvenanceRecord` forensiques (un par appel LLM
    /// Mode C). Vide en keyless. Sérialisables (serde) → persistables/exportables.
    pub llm_records: Vec<efe_nano_sdk::LlmProvenanceRecord>,
    /// **ADR-016 — forensique N17 scellée** : la chaîne HMAC de la forensique LLM
    /// (feuilles SHA-256 de contenu + `n17_root`), scellée au run avec la **même clé
    /// tenant** que la chaîne d'essaim. Vide en keyless/Mode A. C'est ce qui rend la
    /// forensique **durable ET tamper-evidente** une fois persistée par la passerelle —
    /// et re-prouvable hors ligne par `verify_run_offline`.
    pub n17_sealed: efe_swarm_provenance::SealedN17,
}

impl RunOutcome {
    /// Les `AgentCallRecord` bruts à persister (vide si provenance désactivée).
    pub fn records(&self) -> &[efe_swarm_provenance::AgentCallRecord] {
        self.sealed.as_ref().map(|s| s.records.as_slice()).unwrap_or(&[])
    }
    /// **ADR-016** : `n17_root` en hex (la racine de la chaîne forensique), si Mode C.
    pub fn n17_root_hex(&self) -> Option<String> {
        self.n17_sealed.root_hex()
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
    /// **Garde-fou souveraineté (G2)** : un backend self-host **non-local** a été refusé
    /// (egress interdit par défaut). Utilisez un endpoint `127.0.0.1`/`localhost`, ou
    /// activez explicitement `SwarmBuilder::allow_remote_backend(true)`.
    RemoteBackendDenied { endpoint: String },
    /// **Health-check (S.a)** : le backend self-host est **injoignable** (sonde au démarrage,
    /// activée par `require_healthy_backend(true)`). Échec rapide plutôt que N timeouts.
    BackendUnavailable { endpoint: String },
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
            SwarmError::RemoteBackendDenied { endpoint } => {
                write!(f, "backend self-host non-local refusé (souveraineté/egress) : « {endpoint} » ; utilisez 127.0.0.1/localhost ou allow_remote_backend(true)")
            }
            SwarmError::BackendUnavailable { endpoint } => {
                write!(f, "backend self-host injoignable (health-check) : « {endpoint} » ; démarrez le modèle local (ex. `ollama serve`)")
            }
        }
    }
}

impl std::error::Error for SwarmError {}
