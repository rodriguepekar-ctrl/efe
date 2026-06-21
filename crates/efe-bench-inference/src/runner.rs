// SPDX-License-Identifier: UNLICENSED
//! **B1.3 — le runner : exécuter l'essaim via le SDK, lire le `RunOutcome`.**
//!
//! Le runner transforme un dataset + une configuration d'essaim en un faisceau de mesures. Il ne
//! calcule presque rien lui-même : il orchestre, lit ce que le SDK expose, et enregistre.
//!
//! Pour chaque cas : **(a)** construit la tâche **aveugle** (I-BENCH-BLIND) ; **(b)** appelle
//! [`Swarm::run_traced`] — le SDK fait tout le Mode C (recall → génération → agrégation →
//! scellement → re-preuve R0) ; **(c)** lit le [`RunOutcome`] ; **(d)** **note** via le scorer
//! (post-hoc, hors essaim) ; **(e)** enregistre un [`RunRecord`]. Les cas sont indépendants → exécutés
//! en **concurrence bornée** (un sémaphore).
//!
//! **Le coût vient des jetons, pas des centimes** (I-BENCH-PROXY) : le compute local est facturé 0¢,
//! donc `spent_cents` est un signal de gouvernance, pas économique. Les jetons vivent *par appel*
//! dans `outcome.llm_records` (pont N17) — on les **somme** via `total_tokens()`, et on propage le
//! drapeau `is_estimated` (estimé sur backend scripté ; mesuré dès qu'un vrai vLLM rapporte l'usage).
//!
//! **L'unique aiguillage** (I-BENCH-MAC / B1.7) est le backend : sur Mac, un `ScriptedBackend` injecté
//! par le seam ; sur GPU, `ClientMode::SelfHost { endpoint, model }` — tout le reste est identique.

use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tokio::sync::Semaphore;

use efe_swarm_sdk::{
    AgentCallRecord, Budget, ClientMode, LlmProvenanceRecord, OnExceed, RunOutcome, Swarm, SwarmError, TopologySpec,
};

use crate::dataset::{BenchCase, Dataset};
use crate::report::ConfigReport;

/// **L'évidence brute re-prouvable d'un run** (I-BENCH-REPROVE, archi §8.1). Tout ce qu'il faut pour
/// que **n'importe quel détenteur de la clé tenant** re-dérive la preuve hors-ligne via
/// `verify_run_offline` — sans rien re-exécuter, sans réseau, sans faire confiance à un drapeau
/// `verified` stocké. C'est la différence entre « voici mes chiffres » et « voici mes runs,
/// re-prouvez-les ». Non `Serialize` (les `AgentCallRecord` se persistent à part, comme dans la
/// passerelle) ; `Clone` pour être conservé puis re-prouvé. Cf. [`crate::reprove`].
#[derive(Debug, Clone)]
pub struct RunEvidence {
    pub case_id: String,
    /// La chaîne d'essaim scellée (les `AgentCallRecord`).
    pub swarm_records: Vec<AgentCallRecord>,
    /// La forensique N17 par-appel.
    pub llm_records: Vec<LlmProvenanceRecord>,
    /// Les feuilles SHA-256 de la chaîne N17 (ordre canonique).
    pub n17_leaves: Vec<[u8; 32]>,
    /// La racine HMAC de la chaîne N17 (clé tenant). `None` si vide.
    pub n17_root: Option<[u8; 32]>,
}

/// La source d'inférence — **le seul aiguillage** entre la preuve-sur-Mac et la mesure-sur-GPU.
pub enum BenchBackend {
    /// **B1.7 — chemin réel (toujours compilé)** : Mode C self-host contre un endpoint local
    /// (vLLM/Ollama). Le garde-fou de souveraineté du SDK exige un endpoint **local** (boucle
    /// locale) ; sur la box GPU, vLLM écoute en `127.0.0.1` et le harnais tourne sur la box.
    SelfHost { endpoint: String, model: String },
    /// **Mac (seam B0.3, gardé `test-util`)** : backend déterministe injecté — Mode C COMPLET sans
    /// GPU ni réseau. Inerte/absent en prod (I-SEAM).
    #[cfg(feature = "test-util")]
    Scripted(Arc<dyn efe_swarm_sdk::LocalBackend>),
}

/// Une configuration d'essaim à mesurer (un point de la comparaison).
#[derive(Debug, Clone)]
pub struct BenchConfig {
    /// Nom lisible (clé du rapport / du modèle de prix).
    pub name: String,
    /// La topologie exécutée (p.ex. `consensus(petit_modèle, juges, K)` ou un `GenerateVerify` K=1).
    pub spec: TopologySpec,
    /// Injecter `expected = label` pour armer l'oracle **interne** (anti-poison, opt-in, caveat de
    /// fuite documenté). **Par défaut `false`** : la tâche reste aveugle (I-BENCH-BLIND). N'affecte
    /// jamais la note post-hoc, qui est toujours calculée sur le label détenu par le harnais.
    pub inject_oracle_label: bool,
}

impl BenchConfig {
    /// Config aveugle (le défaut) : la tâche ne porte jamais le label.
    pub fn new(name: impl Into<String>, spec: TopologySpec) -> Self {
        Self { name: name.into(), spec, inject_oracle_label: false }
    }

    /// Active l'injection du label pour l'oracle interne (expérience anti-poison, opt-in).
    pub fn with_oracle_label(mut self) -> Self {
        self.inject_oracle_label = true;
        self
    }
}

impl BenchBackend {
    /// Étiquette forensique du backend — **contextualise la latence** (I-BENCH-PROXY / archi §6.3) :
    /// `mean_elapsed_ms` n'est significatif que pour un `selfhost` réel ; sur `scripted` il est
    /// quasi nul et ne doit pas être lu comme une performance. Jamais une clé, jamais un secret.
    pub fn label(&self) -> String {
        match self {
            BenchBackend::SelfHost { endpoint, model } => format!("selfhost:{model}@{endpoint}"),
            #[cfg(feature = "test-util")]
            BenchBackend::Scripted(_) => "scripted".to_string(),
        }
    }
}

/// **Le balayage de K — la « courbe, pas deux points ».** Construit une config de consensus à K fixe
/// par largeur demandée (nommées `swarm-k{K}`). Mesuré sur le même dataset/scorer, le balayage donne
/// la courbe justesse × coût de l'essaim, sur laquelle on pose le point du gros modèle de référence
/// (cf. l'architecture, §7). Chaque K est un point Pareto ; le K fixe garantit un gagnant nu, scorable.
pub fn consensus_sweep(instruction: &str, judges: &[&str], ks: &[usize]) -> Vec<BenchConfig> {
    ks.iter()
        .map(|&k| BenchConfig::new(format!("swarm-k{k}"), TopologySpec::consensus(instruction, judges, k)))
        .collect()
}

/// La mesure d'un cas : justesse + coût + intégrité de preuve, prête pour le rapport et le bundle.
#[derive(Debug, Clone, Serialize)]
pub struct RunRecord {
    pub case_id: String,
    /// Le scorer a-t-il jugé la réponse gagnante correcte (post-hoc, aveugle) ?
    pub correct: bool,
    /// Largeur effective (appels d'agent réellement faits, après sizing/arrêts).
    pub agents_used: usize,
    /// Nombre d'appels LLM forensiques (records N17 ; 0 en keyless / hors Mode C).
    pub n17_calls: usize,
    /// Coût-jetons : somme des `total_tokens()` sur les records N17 (entrée + sortie, canonique).
    pub tokens: u64,
    /// `true` si le coût-jetons est **estimé** (proxy) ; `false` s'il est **mesuré** (vLLM a rapporté
    /// l'usage). I-BENCH-PROXY : le drapeau remonte jusqu'au rapport.
    pub tokens_estimated: bool,
    /// Temps de paroi (chronométré par le SDK). **Significatif seulement sur endpoint réel** : sur
    /// backend scripté il est quasi nul et ne doit pas être lu comme une performance.
    pub elapsed_ms: u64,
    /// Dépense en centimes (gouvernance de budget, **pas** le coût économique : 0¢ en self-host).
    pub spent_cents: u32,
    /// R0 re-prouve la chaîne (le run est attesté). Doit être `true` pour un run propre.
    pub verified: bool,
    /// Racine de provenance d'essaim (hex) — re-vérifiable par un tiers.
    pub provenance_root: Option<String>,
    /// Racine de la chaîne forensique N17 (hex) — Mode C seulement.
    pub n17_root: Option<String>,
    /// Barreau d'attestation de computation **le plus faible** du run (forensique ; p.ex. `R2-LOCAL-TEMOIN`).
    pub attestation: Option<String>,
    /// La sortie scorée (conservée pour le bundle / l'inspection).
    pub winner: Value,
}

impl RunRecord {
    /// Lit le `RunOutcome` et note la réponse contre le label (post-hoc, aveugle au sens où le label
    /// n'a jamais touché les générateurs).
    fn from_outcome(case: &BenchCase, outcome: &RunOutcome) -> Self {
        let winner = outcome.run.output.clone();
        let correct = case.scorer.is_correct(&winner, &case.label);

        // Coût-jetons : depuis les records N17 par-appel (PAS le N17Summary, qui ne porte que
        // l'intégrité des liens). `total_tokens()` = provider si rapporté, sinon estimation client.
        let tokens: u64 = outcome.llm_records.iter().map(|r| u64::from(r.total_tokens())).sum();
        let n17_calls = outcome.llm_records.len();
        // Estimé si AUCUN appel n'a de compte fournisseur (ou s'il n'y a aucun appel : on ne peut
        // pas prétendre « mesuré »). Mesuré seulement si tous les appels portent un compte fournisseur.
        let tokens_estimated =
            outcome.llm_records.is_empty() || outcome.llm_records.iter().any(|r| r.token_usage.is_estimated);
        // Barreau d'attestation le plus faible (le « maillon faible » côté computation).
        let attestation = outcome
            .llm_records
            .iter()
            .min_by_key(|r| r.compute_attestation.strength_rank())
            .map(|r| r.compute_attestation.rung_label().to_string());

        Self {
            case_id: case.id.clone(),
            correct,
            agents_used: outcome.run.accounting.agents_used,
            n17_calls,
            tokens,
            tokens_estimated,
            elapsed_ms: outcome.run.accounting.elapsed_ms,
            spent_cents: outcome.run.budget.spent_cents,
            verified: outcome.run.provenance.verified,
            provenance_root: outcome.run.provenance.root.clone(),
            n17_root: outcome.n17_root_hex(),
            attestation,
            winner,
        }
    }

    /// Un run qui a **échoué** (erreur SDK sur ce cas) : compté faux + non vérifié, sans tuer le banc.
    /// Le cas reste présent (1 record par cas) → l'invariant de comptage du bundle tient ; `verified`
    /// faux fera échouer `bundle.verify()`, honnêtement.
    fn failed(case: &BenchCase, err: &SwarmError) -> Self {
        Self {
            case_id: case.id.clone(),
            correct: false,
            agents_used: 0,
            n17_calls: 0,
            tokens: 0,
            tokens_estimated: true,
            elapsed_ms: 0,
            spent_cents: 0,
            verified: false,
            provenance_root: None,
            n17_root: None,
            attestation: None,
            winner: Value::String(format!("<erreur: {err}>")),
        }
    }
}

/// Extrait l'évidence re-prouvable d'un `RunOutcome` scellé. `None` si la provenance est désactivée
/// (pas de records scellés à re-prouver). Tout est cloné depuis le `RunOutcome` avant qu'il ne soit
/// abandonné — l'évidence devient autonome.
fn evidence_from(case_id: &str, outcome: &RunOutcome) -> Option<RunEvidence> {
    let swarm_records = outcome.sealed.as_ref()?.records.clone();
    Some(RunEvidence {
        case_id: case_id.to_string(),
        swarm_records,
        llm_records: outcome.llm_records.clone(),
        n17_leaves: outcome.n17_sealed.leaf_hashes.clone(),
        n17_root: outcome.n17_sealed.n17_root,
    })
}

/// Construit le `Swarm` pour un backend donné. C'est ici, et nulle part ailleurs, que le chemin
/// scripté (Mac) et le chemin réel (GPU) divergent.
fn build_swarm(backend: &BenchBackend) -> Result<Swarm, SwarmError> {
    match backend {
        BenchBackend::SelfHost { endpoint, model } => Swarm::builder()
            .mode(ClientMode::SelfHost { endpoint: endpoint.clone(), model: model.clone() })
            .build(),
        #[cfg(feature = "test-util")]
        BenchBackend::Scripted(b) => Swarm::builder().with_local_backend(b.clone()).build(),
    }
}

/// Exécute UN cas et le mesure. Renvoie la mesure ET l'évidence re-prouvable (None sur échec ou si la
/// provenance est désactivée). Une erreur SDK par-cas est capturée en [`RunRecord::failed`] (un run
/// brisé ne fait pas tomber tout le banc).
async fn run_case(
    swarm: &Swarm,
    case: &BenchCase,
    config: &BenchConfig,
    budget_cents: u32,
) -> (RunRecord, Option<RunEvidence>) {
    // I-BENCH-BLIND : aveugle par défaut ; oracle-task seulement sur le chemin opt-in.
    let task = if config.inject_oracle_label { case.oracle_task() } else { case.blind_task() };
    let budget = Budget::cents(budget_cents).with_policy(OnExceed::Stop);
    match swarm.run_traced(task, config.spec.clone(), budget).await {
        Ok(outcome) => (RunRecord::from_outcome(case, &outcome), evidence_from(&case.id, &outcome)),
        Err(e) => (RunRecord::failed(case, &e), None),
    }
}

/// **Mesure une configuration sur tout le dataset** (rapport SEUL). Voir
/// [`run_config_with_evidence`] pour aussi obtenir l'évidence re-prouvable.
pub async fn run_config(
    dataset: &Dataset,
    config: &BenchConfig,
    backend: &BenchBackend,
    budget_cents: u32,
    concurrency: usize,
) -> Result<ConfigReport, SwarmError> {
    Ok(run_config_with_evidence(dataset, config, backend, budget_cents, concurrency).await?.0)
}

/// **Mesure une configuration + conserve l'évidence re-prouvable.** Construit un essaim (un seul
/// aiguillage de backend), exécute chaque cas en concurrence bornée, agrège en [`ConfigReport`], et
/// renvoie l'évidence brute par-run pour une re-preuve hors-ligne indépendante (archi §8.1).
///
/// - `budget_cents` : plafond par-run (gouvernance ; en self-host le coût réel est en jetons).
/// - `concurrency` : nombre de cas exécutés simultanément (sur GPU, la profondeur de file que vLLM
///   digère bien ; ramené à ≥ 1).
pub async fn run_config_with_evidence(
    dataset: &Dataset,
    config: &BenchConfig,
    backend: &BenchBackend,
    budget_cents: u32,
    concurrency: usize,
) -> Result<(ConfigReport, Vec<RunEvidence>), SwarmError> {
    let swarm = build_swarm(backend)?;
    let sem = Arc::new(Semaphore::new(concurrency.max(1)));

    let futures_iter = dataset.cases.iter().map(|case| {
        let sem = sem.clone();
        let swarm = &swarm;
        async move {
            // Permis détenu pour toute la durée du run → au plus `concurrency` runs simultanés.
            let _permit = sem.acquire().await.expect("sémaphore jamais fermé");
            run_case(swarm, case, config, budget_cents).await
        }
    });

    let pairs = futures::future::join_all(futures_iter).await;
    let mut records = Vec::with_capacity(pairs.len());
    let mut evidence = Vec::with_capacity(pairs.len());
    for (rec, ev) in pairs {
        records.push(rec);
        if let Some(e) = ev {
            evidence.push(e);
        }
    }
    let mut report = ConfigReport::aggregate(config.name.clone(), records);
    report.backend = backend.label(); // I-BENCH-PROXY : étiquette la source (contextualise la latence).
    Ok((report, evidence))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scorer::ScorerKind;
    use serde_json::json;

    #[test]
    fn failed_record_is_incorrect_and_unverified() {
        let case = BenchCase::new("c1", json!({ "text": "t" }), json!("finance"), ScorerKind::Exact);
        let rec = RunRecord::failed(&case, &SwarmError::EmptySpec("boom"));
        assert!(!rec.correct);
        assert!(!rec.verified, "un run échoué n'est jamais vérifié (le bundle le verra)");
        assert_eq!(rec.case_id, "c1");
        assert!(rec.tokens_estimated, "pas d'appel → on ne prétend pas 'mesuré'");
    }

    #[test]
    fn config_defaults_to_blind() {
        let cfg = BenchConfig::new("c", TopologySpec::consensus("g", &["consistency"], 4));
        assert!(!cfg.inject_oracle_label, "I-BENCH-BLIND : aveugle par défaut");
        assert!(cfg.with_oracle_label().inject_oracle_label, "opt-in explicite seulement");
    }

    #[test]
    fn consensus_sweep_names_one_config_per_k() {
        let configs = consensus_sweep("Classify.", &["consistency"], &[4, 8, 16]);
        let names: Vec<&str> = configs.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["swarm-k4", "swarm-k8", "swarm-k16"], "un point de courbe par K");
        assert!(configs.iter().all(|c| !c.inject_oracle_label), "le balayage reste aveugle");
    }
}
