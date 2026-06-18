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

/// **ADR-SWARM-018 — comment le run s'est terminé** (les *trois plafonds*, un statut).
/// Orthogonale au succès de la provenance : un run peut être tronqué (budget/temps) ET
/// parfaitement re-prouvable. Sérialisée en `snake_case` (contrat de fil stable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTermination {
    /// La topologie a rendu sa sortie sans qu'aucun plafond ne tronque (cas nominal).
    #[default]
    Completed,
    /// **Plafond coût** : le budget ne pouvait plus payer une vague (politique `Stop`) —
    /// résultat partiel, scellé proprement (miroir de `budget_stopped`).
    BudgetStopped,
    /// **Plafond temps — coopératif (I-AD2)** : la sonde de deadline a refusé une vague à
    /// une **frontière nette** ; le partiel est un préfixe propre, re-prouvable (R0) mais
    /// non reproductible (R1, point de coupe wall-clock — écho ADR-017).
    DeadlineExceeded,
    /// **Plafond temps — backstop dur (I-AD3)** : une topologie non coopérative (K fixe,
    /// T2/T3) ou un *hang* a dépassé `deadline + grâce` ; `tokio::timeout` a tranché et le
    /// partiel présent dans le recorder a **quand même** été scellé (point de coupe
    /// arbitraire — re-prouvable, mais plus grossier que le coopératif).
    DeadlineHardAbort,
}

impl RunTermination {
    /// Étiquette stable (logs, dashboard, artefacts).
    pub fn as_str(&self) -> &'static str {
        match self {
            RunTermination::Completed => "completed",
            RunTermination::BudgetStopped => "budget_stopped",
            RunTermination::DeadlineExceeded => "deadline_exceeded",
            RunTermination::DeadlineHardAbort => "deadline_hard_abort",
        }
    }
    /// Le run a-t-il été tronqué par la fenêtre temporelle (coopératif ou backstop) ?
    pub fn is_deadline(&self) -> bool {
        matches!(self, RunTermination::DeadlineExceeded | RunTermination::DeadlineHardAbort)
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
    /// **ADR-018** : comment le run s'est terminé (plafond coût / temps, ou nominal). Champ
    /// additif — `Completed` pour tout run historique (aucun deadline posé). Le signal
    /// riche : `budget_stopped` reste pour rétro-compatibilité, `termination` le subsume.
    #[serde(default)]
    pub termination: RunTermination,
    /// Backend local du run self-host (forensique, pont N17). `None` en keyless (Mode A).
    pub backend: Option<BackendInfo>,
    /// Résumé du pont N17 par-appel (records LLM forensiques + cohérence des liens). `None`
    /// en keyless (Mode A).
    pub n17: Option<N17Summary>,
    /// **ADR-019 (MEM-2)** : l'interaction mémoire inter-runs du run (recall + learn). `None`
    /// si la mémoire est désactivée (chemin historique, I-MEM8). Quand `Some`, dit : la famille
    /// lue, la version + `content_hash` de l'**instantané gelé** (l'épingle, tracée aussi dans
    /// la `provenance_root` via le record MemoryRead, I-MEM2), le warm-start `k_min` appliqué
    /// (borné, I-MEM4), et si le run a **replié** en mémoire (gardé par `is_clean()`, I-MEM3).
    #[serde(default)]
    pub memory: Option<MemorySummary>,
    /// **ADR-019 (MEM-2.b)** : le **pattern d'agrégation gagnant** (`"vote"`/`"borda"`/
    /// `"condorcet"`…) quand la topologie en décide un (consensus) ; `None` sinon (T1/GV,
    /// pipeline, hiérarchique, gossip, débat, ou run tronqué). Rend observable, sur le résultat,
    /// le même verdict que l'event `consensus.decided` — et c'est ce que la mémoire replie dans
    /// le **prior de pattern** (`PatternPrior`). Champ additif (`None` pour tout run historique).
    #[serde(default)]
    pub decided_pattern: Option<String>,
}

/// **ADR-019 (MEM-2)** — l'interaction mémoire d'un run : ce qui a été lu (recall) et écrit
/// (learn). Le miroir, côté mémoire, de [`N17Summary`] pour la forensique LLM. Sérialisé dans
/// la réponse du run + le flux ; rend l'apprentissage **visible** (comme ADR-018 a rendu la
/// terminaison visible).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemorySummary {
    /// Un instantané a-t-il été lu (recall activé + famille résolue) ?
    pub recalled: bool,
    /// La famille de tâche (empreinte hex) — l'index de la mémoire consultée.
    pub family: String,
    /// La version de l'instantané lu (= nombre de replis pliés ; 0 = famille neuve).
    pub version: u64,
    /// Le `content_hash` (hex) de l'instantané gelé — **l'épingle** (I-MEM1) ; identique à ce
    /// qu'ancre le record MemoryRead dans la racine, et ce que R1 ré-épinglera (I-MEM7).
    pub content_hash: String,
    /// Le `k_min` effectivement amorcé par la mémoire (borné aux bornes du run, I-MEM4).
    /// `None` ⇒ pas d'amorce (pas de donnée, ou topologie non adaptative).
    pub warm_start_k_min: Option<usize>,
    /// Le run a-t-il **replié** en mémoire (propre + verified + learn activé) — I-MEM3 ?
    pub learned: bool,
}

impl SwarmRun {
    /// Le run est-il intègre ET non tronqué (ni par le coût, ni par le temps) ?
    /// La provenance se re-prouve, et aucun plafond n'a interrompu : `termination` subsume
    /// l'ancien `!budget_stopped` et l'étend au deadline (un run deadliné est partiel, donc
    /// pas `clean` — même s'il reste parfaitement re-prouvable, cf. ADR-018).
    pub fn is_clean(&self) -> bool {
        (!self.provenance.enabled || self.provenance.verified)
            && matches!(self.termination, RunTermination::Completed)
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
    /// **Garde-fou G2-remote (RC-1)** : un backend distant a été autorisé (`allow_remote_backend`)
    /// mais son endpoint n'est **pas épinglé**. Tout distant doit être épinglé (empreinte
    /// cert/SPKI) — aucun distant en clair, aucun distant non pinné. Déclarez l'épingle via
    /// `SwarmBuilder::pin_remote_endpoint(host, spki_fingerprint)`. Voir l'architecture §5.
    RemoteEndpointNotPinned { endpoint: String },
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
            SwarmError::RemoteEndpointNotPinned { endpoint } => {
                write!(f, "backend distant NON ÉPINGLÉ refusé (G2-remote) : « {endpoint} » ; épinglez-le via pin_remote_endpoint(host, empreinte cert/SPKI) — aucun distant non pinné")
            }
            SwarmError::BackendUnavailable { endpoint } => {
                write!(f, "backend self-host injoignable (health-check) : « {endpoint} » ; démarrez le modèle local (ex. `ollama serve`)")
            }
        }
    }
}

impl std::error::Error for SwarmError {}
