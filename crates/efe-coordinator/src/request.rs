// SPDX-License-Identifier: UNLICENSED
//! Types I/O des 3 missions du Coordinator.
//!
//! Le Coordinator expose 3 entrées publiques :
//!
//! - [`MissionARequest`] / [`MissionAResponse`] — pré-mâcher pour 1 LLM.
//! - [`MissionBRequest`] / [`MissionBResponse`] — fan-out N LLMs + vote.
//! - [`SpeculativeRequest`] / [`SpeculativeResponse`] — drafter/verifier.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata, TaskClassScoringTable};

use crate::failure::FailureMode;
use crate::metrics::ChewingMetrics;

/// Requête utilisateur pour la Mission A.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionARequest {
    /// Texte saisi par l'humain.
    pub user_text: String,

    /// Catégorie sémantique (utilisée par le Router pour choisir le LLM).
    pub task_class: TaskClass,

    /// LLMs candidats (au moins 1).
    pub available_llms: Vec<String>,

    /// Métadonnées par LLM (coût/latence/qualité — consommées par le Router).
    pub llm_metadata: HashMap<String, LlmMetadata>,

    /// Contraintes budgétaires.
    pub budget: Budget,

    /// Sources candidates pour le Verifier (≤ 50).  Optionnel : si absent,
    /// le Verifier est skippé (mode "no fact-check").
    #[serde(default)]
    pub candidate_sources: Vec<CandidateSourceRef>,

    /// Deadline globale de la mission.
    #[serde(default = "default_deadline")]
    pub max_total_latency: Duration,

    /// **Sprint S6 R2** — Table de scoring statique optionnelle.
    ///
    /// Si présent, override `coord.clients.router` (qui utilise des poids
    /// uniformes 1.0/1.0/1.0) avec un `RouterClient::for_task_class(...)`
    /// construit à la volée dans `run_mission_a`. C'est ce qui permet à
    /// FactualQa de router vers Gemini (cost-driven) tandis que Code route
    /// vers Claude (quality-driven), à partir de la même invocation
    /// utilisateur.
    ///
    /// `#[serde(skip)]` : la table est versionnée + chargée en mémoire au
    /// démarrage de l'app. Inutile de la roundtripper dans les payloads
    /// JSON ; l'audit chain trace déjà le `router.reason` (cf. S5).
    ///
    /// `Arc` : permet le partage cheap entre Mission A et Mission B sans
    /// clone profond, et entre threads (fan-out parallèle Mission B).
    #[serde(skip)]
    pub task_class_scoring_table: Option<Arc<TaskClassScoringTable>>,
}

impl MissionARequest {
    /// **Sprint S6 R2** — Helper : construit une `MissionARequest` avec la
    /// table de scoring par défaut (3 LLMs : Claude + Gemini + GPT, 6
    /// task_class calibrés).
    ///
    /// Évite au caller de remplir manuellement `llm_metadata` et `budget`.
    /// `budget` est défini à 50 ¢ / 30 s / qualité 0 (non-contraignant).
    ///
    /// Le caller peut toujours patcher les champs après construction :
    ///
    /// ```ignore
    /// let mut req = MissionARequest::with_default_scoring(
    ///     "Écris une fonction Rust pour fibonacci".to_string(),
    ///     TaskClass::Code,
    ///     vec!["claude_…".into(), "gemini_…".into(), "gpt_…".into()],
    /// );
    /// req.budget.max_cost_cents = 10;  // override : cap budget
    /// req.candidate_sources = my_sources;  // ajoute fact-check
    /// ```
    #[must_use]
    pub fn with_default_scoring(
        user_text: String,
        task_class: TaskClass,
        available_llms: Vec<String>,
    ) -> Self {
        let table = TaskClassScoringTable::default_3llm_table();
        let llm_metadata = table.build_metadata_map(&available_llms, task_class);
        Self {
            user_text,
            task_class,
            available_llms,
            llm_metadata,
            budget: Budget {
                max_cost_cents: 50,
                // R3.S1 : 0 → _resolved() fallback à cents × 1000.
                max_cost_microcents: 0,
                max_latency_ms: 30_000,
                min_quality_score: 0.0,
            },
            candidate_sources: vec![],
            max_total_latency: Duration::from_secs(30),
            task_class_scoring_table: Some(Arc::new(table)),
        }
    }
}

/// Réponse finale produite par la Mission A.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionAResponse {
    /// Texte généré par le LLM choisi (concaténation des tokens streamés).
    pub final_text: String,

    /// LLM réellement appelé.
    pub llm_used: String,

    /// Coût constaté (cents).
    ///
    /// **LEGACY** : utiliser `cost_microcents` (R3.S1) pour la précision sub-cent.
    pub cost_cents: u32,

    /// **R3.S1** : coût constaté en microcents (1 µ¢ = 1/1000 cent).
    ///
    /// `#[serde(default)]` pour rétrocompat avec les `MissionAResponse`
    /// sérialisés pré-R3 (champ absent → 0). Le workflow peut utiliser le
    /// helper `microcents_or_legacy()` pour fallback transparent.
    #[serde(default)]
    pub cost_microcents: u64,

    /// Latence end-to-end (ms).
    pub latency_ms: u64,

    /// Verdicts de fact-checking (vide si pas de sources).
    pub verdicts: Vec<FactCheck>,

    /// Hash BLAKE3 de la chaîne d'audit complète (preuve d'intégrité).
    pub audit_chain_hash: String,

    /// Métriques de pré-mâchage (Sprint S4 R2) — `None` si la mission n'a pas
    /// calculé les métriques (ex: tests offline avec mocks, ou versions
    /// antérieures de Mission A).
    ///
    /// `#[serde(default)]` garantit la backward-compat : les `MissionAResponse`
    /// sérialisés sans ce champ continuent à désérialiser correctement.
    #[serde(default)]
    pub chewing_metrics: Option<ChewingMetrics>,

    /// **Sprint S8 R2** — Si un fallback LLM a été déclenché, contient le
    /// LLM cible effectivement utilisé. `None` pour le cas nominal (LLM
    /// `selected` du router a marché du premier coup).
    ///
    /// Permet à l'utilisateur de tracer rétroactivement : "ma mission a
    /// utilisé Gemini alors que le router avait choisi Claude → Claude
    /// était down ou rate limited".
    #[serde(default)]
    pub fallback_chain_used: Option<String>,

    /// **Sprint S8 R2** — Mode de failure observé pendant la mission.
    /// `FailureMode::None` pour cas nominal.
    #[serde(default)]
    pub failure_mode: FailureMode,
}

/// Requête pour la Mission B (multi-LLM + vote).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionBRequest {
    /// Texte saisi.
    pub user_text: String,

    /// Catégorie sémantique.
    pub task_class: TaskClass,

    /// LLMs candidats (≥ 2 — sinon fallback Mission A).
    pub llms: Vec<String>,

    /// Métadonnées par LLM.
    pub llm_metadata: HashMap<String, LlmMetadata>,

    /// Poids `w_l` par LLM utilisés dans le vote ([`crate::vote::VoteEngine`]).
    /// Si absent, poids uniformes (1.0 partout).
    #[serde(default)]
    pub vote_weights: HashMap<String, f32>,

    /// Quorum requis (par défaut `ceil(|llms| / 2)`).
    #[serde(default)]
    pub quorum: Option<usize>,

    /// Sources candidates pour le verifier final.
    #[serde(default)]
    pub candidate_sources: Vec<CandidateSourceRef>,

    /// Deadline.
    #[serde(default = "default_deadline")]
    pub max_total_latency: Duration,
}

/// Réponse de la Mission B.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionBResponse {
    /// Texte gagnant du vote (concaténation des tokens).
    pub final_text: String,

    /// LLM gagnant.
    pub winner_llm: String,

    /// Score du gagnant (somme pondérée).
    pub winner_score: f32,

    /// Score de chaque LLM ayant participé (ordre = paramètres).
    pub per_llm_scores: HashMap<String, f32>,

    /// Métriques de pré-mâchage **mutualisé** (Sprint S5 R2) — `None` si la
    /// mission n'a pas calculé les métriques (mocks offline anciens).
    ///
    /// En Mission B, le compressor tourne **1× seulement** mais son output est
    /// envoyé aux N LLMs en parallèle. Les économies sont donc multipliées :
    /// `savings_tokens × N` est l'économie cumulée sur l'ensemble des LLMs.
    ///
    /// - `llm_latency_ms` = `max` des N latences (fan-out parallèle, pas somme)
    /// - `llm_tokens_generated` = somme des tokens output des N LLMs
    /// - `selected_llm` = `winner_llm` (LLM gagnant du vote Symphony, pas du
    ///   router — Mission B n'utilise pas le router)
    /// - `router_*` = vides (Mission B appelle tous les LLMs sans choisir)
    ///
    /// `#[serde(default)]` garantit la backward-compat avec des
    /// `MissionBResponse` sérialisés pré-S5.
    #[serde(default)]
    pub chewing_metrics: Option<ChewingMetrics>,

    /// Coût agrégé (cents).
    ///
    /// **LEGACY** : utiliser `total_cost_microcents` (R3.S1) pour la précision
    /// sub-cent — en Mission B la somme des coûts des N LLMs amplifie l'erreur
    /// d'arrondi cents (notamment quand 1 LLM = Gemini Flash).
    pub total_cost_cents: u32,

    /// **R3.S1** : coût agrégé en microcents (somme des `cost_microcents` des
    /// N LLMs participants).  Précision conservée même pour Mission B avec
    /// Gemini Flash dans la vote.
    #[serde(default)]
    pub total_cost_microcents: u64,

    /// Latence end-to-end.
    pub latency_ms: u64,

    /// Fact-checks éventuels.
    pub verdicts: Vec<FactCheck>,

    /// Hash audit chain.
    pub audit_chain_hash: String,

    /// **Sprint S8 R2** — `true` si quorum atteint MAIS moins de N LLMs
    /// ont effectivement voté (1 ou 2 LLMs sur N ont fail).
    ///
    /// Le frontend peut afficher un badge "low confidence" sans casser l'UX
    /// — la mission a quand même retourné une réponse.
    #[serde(default)]
    pub vote_degraded: bool,

    /// **Sprint S8 R2** — LLMs ayant échoué pendant le fan-out parallèle.
    /// Chaque entry : `(provider_id, reason)`. Vide si tous les LLMs ont
    /// répondu (cas nominal).
    #[serde(default)]
    pub failed_llms: Vec<(String, String)>,

    /// **Sprint S8 R2** — Mode de failure observé pendant la mission.
    #[serde(default)]
    pub failure_mode: FailureMode,
}

/// Requête pour la Mission speculative (drafter LLM cheap + verifier LLM premium).
///
/// **Sprint S7 R2** — Pattern Speculative RAG :
/// 1. Drafter (LLM cheap, e.g. Gemini Flash) produit la réponse candidate.
/// 2. Si elle est courte (≤ `max_words_for_quick_accept`) → accept direct.
///    Verifier NON appelé (HOT PATH, savings ~98 %).
/// 3. Sinon → verifier (LLM premium, e.g. Claude) appelé pour arbitrage
///    Jaccard (SLOW PATH, savings 0).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeculativeRequest {
    /// Texte d'entrée.
    pub user_text: String,

    /// Catégorie.
    pub task_class: TaskClass,

    /// **Sprint S7 R2** — ID du drafter (cheap LLM, e.g. `"gemini_gemini-2.5-flash"`).
    /// Défaut `"drafter"` pour backward-compat S2-S6.
    #[serde(default = "default_drafter_id")]
    pub drafter_llm: String,

    /// LLM "verifier" (gros — Claude/GPT-4/...).
    pub verifier_llm: String,

    /// Métadonnées du verifier.
    pub verifier_metadata: LlmMetadata,

    /// Budget.
    pub budget: Budget,

    /// Nombre max de tokens à drafter en spéculation.
    #[serde(default = "default_speculative_window")]
    pub max_draft_tokens: u32,

    /// Deadline globale.
    #[serde(default = "default_deadline")]
    pub max_total_latency: Duration,

    /// **Sprint S7 R2** — Seuil de similarité Jaccard pour slow path.
    /// Défaut 0.7.
    #[serde(default = "default_accept_threshold")]
    pub accept_threshold: f32,

    /// **Sprint S7 R2** — Nombre max de mots dans le draft pour déclencher
    /// le **hot path** (accept sans verifier). `0` désactive le hot path
    /// → toujours slow path (backward-compat S2-S6). Défaut 10.
    #[serde(default = "default_max_words_for_quick_accept")]
    pub max_words_for_quick_accept: usize,
}

impl SpeculativeRequest {
    /// **Sprint S7 R2** — Helper : construit avec drafter cheap + verifier
    /// premium, `verifier_metadata` injecté depuis la table S6.
    ///
    /// ```ignore
    /// let req = SpeculativeRequest::with_drafter_and_verifier(
    ///     "Capital of France?".into(),
    ///     TaskClass::FactualQa,
    ///     "gemini_gemini-2.5-flash".into(),
    ///     "claude_claude-sonnet-4-6".into(),
    /// );
    /// ```
    #[must_use]
    pub fn with_drafter_and_verifier(
        user_text: String,
        task_class: TaskClass,
        drafter_llm: String,
        verifier_llm: String,
    ) -> Self {
        use efe_nano_app_coordinator::types::TaskClassScoringTable;
        let table = TaskClassScoringTable::default_3llm_table();
        let verifier_metadata = table.build_llm_metadata(&verifier_llm, task_class);
        Self {
            user_text,
            task_class,
            drafter_llm,
            verifier_llm,
            verifier_metadata,
            budget: Budget {
                max_cost_cents: 50,
                // R3.S1 : 0 → _resolved() fallback à cents × 1000.
                max_cost_microcents: 0,
                max_latency_ms: 30_000,
                min_quality_score: 0.0,
            },
            max_draft_tokens: default_speculative_window(),
            max_total_latency: default_deadline(),
            accept_threshold: default_accept_threshold(),
            max_words_for_quick_accept: default_max_words_for_quick_accept(),
        }
    }
}

/// Réponse spéculative.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeculativeResponse {
    /// Texte final (le verifier a le dernier mot).
    pub final_text: String,

    /// Texte qui avait été drafté avant vérification.
    pub draft_text: String,

    /// Drafter accepté en l'état (= verifier validé sans changement) ?
    pub draft_accepted: bool,

    /// Nombre de tokens du draft acceptés tels quels.
    pub tokens_accepted: u32,

    /// Nombre de tokens regénérés par le verifier.
    pub tokens_regenerated: u32,

    /// Gain de latence vs. baseline (ms négatif si la spéculation a coûté plus cher).
    pub latency_saved_ms: i64,

    /// Latence end-to-end.
    pub latency_ms: u64,

    /// Hash audit chain.
    pub audit_chain_hash: String,

    /// **Sprint S7 R2** — Le verifier API a-t-il été appelé ?
    ///
    /// - `true` (HOT PATH ~80 %) : drafter seul, verifier API skippée → vrai savings
    /// - `false` (SLOW PATH ~20 %) : verifier appelé, savings 0
    ///
    /// `#[serde(default)]` pour backward-compat.
    #[serde(default)]
    pub verifier_skipped: bool,

    /// **Sprint S7 R2** — Métriques de pré-mâchage adaptées Mission Spec.
    ///
    /// Champs Spec-spécifiques :
    /// - `verifier_skipped: Some(bool)` — VRAI savings ?
    /// - `spec_drafter_latency_ms` + `spec_verifier_latency_ms` (0 si skip)
    /// - `spec_savings_cents` — estimation cents économisés
    ///
    /// `None` pour les anciens `SpeculativeResponse` sérialisés pré-S7.
    #[serde(default)]
    pub chewing_metrics: Option<ChewingMetrics>,

    /// **Sprint S7 R2** — Coût total constaté de la mission speculative.
    ///
    /// - HOT PATH : `cost_cents = drafter.cost_cents` (seul drafter facturé)
    /// - SLOW PATH : `cost_cents = drafter + verifier` (les deux facturés)
    ///
    /// Symétrique avec `MissionAResponse.cost_cents` et
    /// `MissionBResponse.total_cost_cents`. Permet à un caller de comparer
    /// les coûts entre missions et de quantifier empirically le savings :
    ///
    /// ```text
    /// savings = baseline_cost_cents - resp.cost_cents
    /// ```
    ///
    /// `#[serde(default)]` pour backward-compat S2-S6.
    #[serde(default)]
    pub cost_cents: u32,

    /// **R3.S1** : coût total en microcents (1 µ¢ = 1/1000 cent).
    ///
    /// Particulièrement important en HOT PATH (Speculative bypass verifier) :
    /// le drafter étant typiquement Gemini Flash, le coût constaté est de
    /// l'ordre de quelques microcents et n'apparaît PAS dans `cost_cents`
    /// (arrondi à 1 cent par sécurité). `cost_microcents` montre le vrai
    /// chiffre.
    ///
    /// Formule :
    /// - HOT PATH : `cost_microcents = drafter.cost_microcents`
    /// - SLOW PATH : `cost_microcents = drafter.cost_microcents + verifier.cost_microcents`
    ///
    /// `#[serde(default)]` pour backward-compat S2-S6/S7-S10 (pré-R3).
    #[serde(default)]
    pub cost_microcents: u64,
}

/// Verdict de fact-check résumé (alias UI-friendly).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactCheck {
    /// Claim verbatim qui a été vérifié.
    pub claim: String,
    /// Verdict (`SUPPORTED`, `UNSUPPORTED`, `CONTRADICTED`, `INSUFFICIENT_EVIDENCE`).
    pub verdict: String,
    /// Sévérité (`INFO`/`WARN`/`ERROR`).
    pub severity: String,
    /// ID de la source la mieux matchée (None si UNSUPPORTED).
    pub best_source_id: Option<String>,
    /// Similarité 0..1.
    pub similarity: f32,
}

/// Référence à une source candidate (utilisée par les Missions A et B).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateSourceRef {
    /// ID unique.
    pub id: String,
    /// Texte de la source.
    pub text: String,
}

fn default_deadline() -> Duration {
    Duration::from_secs(30)
}

const fn default_speculative_window() -> u32 {
    128
}

/// **Sprint S7 R2** — Drafter ID par défaut pour backward-compat S2-S6.
fn default_drafter_id() -> String {
    "drafter".to_string()
}

/// **Sprint S7 R2** — Seuil Jaccard slow path par défaut.
const fn default_accept_threshold() -> f32 {
    0.7
}

/// **Sprint S7 R2** — Mots max pour hot path par défaut.
const fn default_max_words_for_quick_accept() -> usize {
    10
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_app_coordinator::types::router::TaskClass;

    #[test]
    fn mission_a_roundtrips() {
        let req = MissionARequest {
            user_text: "what is the Fed rate?".into(),
            task_class: TaskClass::FactualQa,
            available_llms: vec!["claude".into()],
            llm_metadata: HashMap::new(),
            budget: Budget {
                max_cost_cents: 100,
                // R3.S1 : 0 → _resolved() fallback à cents × 1000.
                max_cost_microcents: 0,
                max_latency_ms: 5000,
                min_quality_score: 0.5,
            },
            candidate_sources: vec![],
            max_total_latency: Duration::from_secs(10),
            task_class_scoring_table: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        let back: MissionARequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.user_text, "what is the Fed rate?");
    }

    #[test]
    fn mission_b_default_quorum_is_none() {
        let req = MissionBRequest {
            user_text: "x".into(),
            task_class: TaskClass::Analysis,
            llms: vec!["claude".into(), "gpt4".into(), "gemini".into()],
            llm_metadata: HashMap::new(),
            vote_weights: HashMap::new(),
            quorum: None,
            candidate_sources: vec![],
            max_total_latency: Duration::from_secs(10),
        };
        assert!(req.quorum.is_none());
    }

    #[test]
    fn speculative_default_window_is_128() {
        let json = r#"{"user_text":"x","task_class":"FACTUAL_QA","verifier_llm":"claude","verifier_metadata":{"cost_input_per_million":1000,"cost_output_per_million":1000,"p50_latency_ms":100,"p99_latency_ms":1000,"success_rate":0.99,"capability_scores":{},"circuit_open":false},"budget":{"max_cost_cents":100,"max_latency_ms":1000,"min_quality_score":0.5}}"#;
        let req: SpeculativeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.max_draft_tokens, 128);
    }
}
