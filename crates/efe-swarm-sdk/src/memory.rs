// SPDX-License-Identifier: UNLICENSED
//! # `memory` — câblage de la mémoire inter-runs dans le run (ADR-SWARM-019, lot MEM-2)
//!
//! MEM-1 a livré le socle PUR (`efe-swarm-memory` : cellules, fold, chaîne `memory_root`,
//! `MemoryStore`). MEM-2 le **branche dans `run_traced`** sous le principe d'ADR-019 : *la
//! mémoire est une ENTRÉE épinglée/hachée/tracée*. Trois temps, tous **opt-in** (sans bloc
//! mémoire, le run est byte-identique au chemin historique — I-MEM8) :
//!
//! 1. **Recall** : en tête de run, on calcule la `FamilyKey`, on lit un **instantané gelé**
//!    (`MemoryStore::snapshot`), et l'on **trace** la lecture par un record [`AgentRole::Memory`]
//!    capturé dans la chaîne de provenance (I-MEM2) — la `provenance_root` dépend donc de ce
//!    qui a été lu.
//! 2. **Influence BORNÉE** (I-MEM4) : le **prior de taille** amorce `k_min` (warm-start),
//!    **clampé aux bornes du run** par le resolver — la mémoire amorce, ne gouverne jamais.
//! 3. **Learn** : après scellement+vérification, **si et seulement si** le run est propre
//!    (`SwarmRun::is_clean()` — la porte I-MEM3), on replie un [`RunSummary`].
//!
//! ## Périmètre — MEM-2 puis MEM-2.b
//! **MEM-2** captait le **fiablement exposé** d'un `SwarmRun` : `k_used` (prior de taille →
//! warm-start) et le **gagnant** comme exemplaire (stocké, mais pas encore relu).
//!
//! **MEM-2.b** ferme deux des trois lectures différées :
//! - **Pattern (populate)** — le **pattern gagnant** + l'**escalade** sont désormais portés par
//!   `SwarmRun::decided_pattern` (alimenté par le consensus via `TopologyOutput`), donc repliés
//!   dans un **vrai** `PatternPrior` (auparavant `None`/vide). Sa lecture-retour (biaiser le
//!   sélecteur de pattern) — désormais BÂTIE (read-back A) : `PatternSpec::Adaptive` rend
//!   atteignable le sélecteur, et `Resolver::biased_selector` le biaise (θ_eff clampé, I-PAT1).
//! - **Exemplaires (read)** — les meilleurs exemplaires recall-és sont **injectés en few-shot**
//!   dans le prompt des générateurs **Mode C** (`llm.rs::gen_prompt`), bornés à [`FEWSHOT_N`] et
//!   re-prouvables (l'instantané d'où ils viennent est épinglé + tracé, I-MEM2/I-MEM7).
//!
//! **Read-backs A & B (les deux structures neuves)** ferment les deux lectures qui restaient :
//! - **A — biais de pattern** : `PatternSpec::Adaptive` (spec/DTO `"adaptive"`) + le sélecteur
//!   biaisé par le prior d'escalade/dispersion (clampé, sous veto des candidats, I-PAT1-5).
//! - **B — routage par réputation** : la **réputation par-variante** n'est plus dégénérée —
//!   l'identité d'une variante est son **INDICE** (`generator_{i:03}`, un profil d'échantillonnage
//!   stable), calculée à l'attribution (`TopologyOutput::agent_contributions` → `agents`), SANS
//!   toucher au métrage/ancrage (le record porte déjà le marqueur `variant`, moat préservé). La
//!   lecture R-a pondère l'agrégation (poids UCB clampés, sous veto), byte-identique à l'uniforme
//!   (I-REP1-6). Jamais fabriqué : hors consensus, `agents` reste vide.
//!
//! **MEM-2.c** ferme la 5ᵉ cellule, la **mémoire d'échec** (auparavant bâtie mais jamais
//! alimentée) : chaque run propre compte au niveau **FAMILLE** (`failure_key` = la famille), et
//! `failed = !produced_acceptable` — où **la topologie** décide l'acceptabilité (`TopologyOutput`),
//! seule à connaître sa sémantique : GV/T1 ⇒ un candidat a passé le vérificateur (≠ `best_any` non
//! vérifié) ; consensus/pipeline/… ⇒ la réponse est présente (`value`|`text`|nue, non vide). Le SDK
//! ne **devine** plus l'échec depuis la forme du gagnant (faux négatifs GV + sorties `text` vides
//! corrigés, revue adversariale). Un échec n'est PAS déposé comme exemplaire few-shot. La cellule
//! est observable via `GET /memory/:family` (`failures`). La **décroissance temporelle** du
//! compteur reste différée (une décroissance horloge briserait le déterminisme octet — différé sain).

use std::sync::Arc;

use serde_json::{json, Value};

use efe_nano_sdk::AgentId;
use efe_swarm_memory::{Exemplar, FamilyKey, FamilyMode, MemorySnapshot, MemoryStore, RunSummary};
use efe_swarm_provenance::{blake3_value, AgentKind, AgentRole, CallCapture};

use crate::result::SwarmRun;
use crate::spec::TopologySpec;

/// La **politique mémoire** d'un run (opt-in). Défaut : recall + learn, empreinte structurelle.
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    /// Lire l'instantané (warm-start + trace MemoryRead) en tête de run.
    pub recall: bool,
    /// Replier le run propre en mémoire après scellement (porte I-MEM3).
    pub learn: bool,
    /// L'empreinte de famille (défaut : structurelle, souveraine).
    pub mode: FamilyMode,
    /// Famille imposée (sinon dérivée de la tâche + topologie). Utile pour le benchmark.
    pub family_override: Option<FamilyKey>,
    /// **ADR-019 (MEM-3, I-MEM7)** : épingle une version PASSÉE de la mémoire (au lieu de la
    /// courante). `Some(v)` ⇒ le recall lit `snapshot_at(v)` — l'instantané EXACT d'alors, pour
    /// **reproduire** un run (extension d'ADR-017 : déterministe sachant l'instantané). `None`
    /// ⇒ version courante (le cas nominal).
    pub pin_version: Option<u64>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            recall: true,
            learn: true,
            mode: FamilyMode::Structural,
            family_override: None,
            pin_version: None,
        }
    }
}

impl MemoryConfig {
    /// Lit la mémoire mais n'apprend pas (run gouverné par l'expérience, sans la modifier).
    pub fn recall_only() -> Self {
        Self { learn: false, ..Default::default() }
    }
    /// Apprend mais ne lit pas (accumule sans s'auto-influencer — utile pour amorcer).
    pub fn learn_only() -> Self {
        Self { recall: false, ..Default::default() }
    }
}

/// La **poignée mémoire** branchée sur un `Swarm` : le store + la politique. `Clone` bon
/// marché (le store est un `Arc`). `None` sur un `Swarm` ⇒ mémoire OFF (chemin historique).
#[derive(Clone)]
pub struct MemoryHandle {
    pub store: Arc<dyn MemoryStore>,
    pub config: MemoryConfig,
}

impl MemoryHandle {
    /// Poignée avec la politique par défaut (recall + learn, structurelle).
    pub fn new(store: Arc<dyn MemoryStore>) -> Self {
        Self { store, config: MemoryConfig::default() }
    }
    /// Poignée avec une politique explicite.
    pub fn with_config(store: Arc<dyn MemoryStore>, config: MemoryConfig) -> Self {
        Self { store, config }
    }
}

/// L'empreinte de famille d'un run, dérivée de `(tâche, signature de topologie, mode)`.
/// `family_override` court-circuite (benchmark sur une famille fixe).
pub(crate) fn family_for(task: &Value, spec: &TopologySpec, config: &MemoryConfig) -> FamilyKey {
    if let Some(f) = &config.family_override {
        return f.clone();
    }
    let (kind, agents) = spec.family_signature();
    efe_swarm_memory::family_key(task, &kind, &agents, config.mode)
}

/// Le **record MemoryRead** (I-MEM2) : ce qu'un run a LU de la mémoire, à ancrer dans la
/// `provenance_root` via `recorder.capture`. Input = l'identité de famille ; output = la
/// version + le `content_hash` de l'instantané (l'épingle) + l'amorce `k_min` appliquée.
/// Record `Memory`/`ModeA` sans écriture (lecture pure) : il n'altère pas le DAG, il ajoute la
/// preuve de la lecture à la racine.
pub(crate) fn memory_read_capture(
    family: &FamilyKey,
    snap: &MemorySnapshot,
    warm_k_min: Option<usize>,
    exemplars_recalled: usize,
) -> CallCapture {
    let input = json!({ "family": family.as_str(), "op": "recall" });
    let output = json!({
        "version": snap.version,
        "content_hash": snap.content_hash_hex(),
        "warm_start_k_min": warm_k_min,
        // MEM-2.b : combien d'exemplaires few-shot ont été recall-és + rendus disponibles aux
        // générateurs Mode C (l'épingle content_hash ci-dessus les couvre déjà ; ce compte rend
        // l'INFLUENCE explicitement visible dans la racine, sans jamais fabriquer de donnée).
        "exemplars_recalled": exemplars_recalled,
        // read-backs A/B : les SIGNAUX BRUTS lus pour biaiser le pattern + router par réputation.
        // L'épingle content_hash ci-dessus les couvre déjà (les influences DÉRIVÉES — θ_eff, poids —
        // sont des fonctions déterministes de l'instantané, donc re-dérivables, I-PAT3/I-REP4) ; on
        // les rend ici EXPLICITES dans la racine, sans jamais fabriquer de donnée.
        "pattern_escalation_rate_milli": snap.cells.pattern.escalation_rate_milli(),
        "pattern_total": snap.cells.pattern.total,
        "reputation_agents": snap.cells.reputation.agents.len(),
    });
    CallCapture::new(
        AgentId::new("memory_recall").expect("agent id mémoire valide"),
        AgentRole::Memory,
        AgentKind::ModeA,
        &input,
        &output,
    )
}

/// Construit le [`RunSummary`] que la mémoire replie, depuis un `SwarmRun`. Capte le
/// **fiablement exposé** : `k_used` (le K, via `quality.candidates`), le **gagnant** comme
/// exemplaire few-shot (content-addressed), et — depuis MEM-2.b — le **pattern gagnant** +
/// l'**escalade** (désormais portés par `SwarmRun::decided_pattern` / le param `escalated`,
/// alimentés par le consensus via `TopologyOutput`). La **réputation par-variante** (read-back B)
/// vient de `contributions` (`TopologyOutput::agent_contributions`) : l'identité d'une variante est
/// son INDICE (`generator_{i:03}`, profil d'échantillonnage stable) — non plus dégénérée. Vide hors
/// consensus (pas de variantes attribuables) → le fold ④ ne touche rien.
/// `clean` reflète `is_clean()` ; sur le chemin learn (gardé), toujours vrai, mais porté pour
/// la traçabilité.
pub(crate) fn summary_from_run(
    run: &SwarmRun,
    escalated: bool,
    produced_acceptable: bool,
    contributions: &[efe_swarm_topology::AgentContribution],
    family: &FamilyKey,
) -> RunSummary {
    // Le gagnant : la valeur agrégée (`output.value` pour consensus/T1 ; sinon la sortie nue).
    let winner = run.output.get("value").cloned().unwrap_or_else(|| run.output.clone());
    // **Mémoire d'échec (MEM-2.c)** : a-t-il produit une réponse ACCEPTABLE ? Décidé PAR la
    // topologie (`TopologyOutput::produced_acceptable`), qui seule connaît sa sémantique : GV/T1
    // ⇒ un candidat a passé le vérificateur (≠ `best_any` non vérifié, non-nul mais faux) ;
    // consensus/pipeline/… ⇒ la réponse est présente (`value`|`text`|nue, non vide). Le SDK ne
    // DEVINE plus l'échec depuis la forme du gagnant (faux négatifs corrigés : revue adversariale).
    let failed = !produced_acceptable;
    // On ne dépose un exemplaire few-shot QUE pour un vrai gagnant : on ne few-shot jamais un échec.
    let exemplars = if failed {
        Vec::new()
    } else {
        vec![Exemplar {
            content_hash: blake3_value(&winner).to_hex(),
            preview: preview_of(&winner),
            // Un run propre : son gagnant est la réponse acceptée → pleine qualité. La graduation
            // fine (confiance d'agrégation) viendra quand `SwarmRun` l'exposera (MEM-3).
            quality_milli: 1000,
        }]
    };
    RunSummary {
        clean: run.is_clean(),
        k_used: run.quality.candidates as u32,
        // MEM-2.b : le VRAI prior de pattern (consensus). `None` hors consensus → le fold
        // ignore simplement la cellule pattern (cf. `fold_into` ②).
        winning_pattern: run.decided_pattern.clone(),
        escalated,
        exemplars,
        // **read-back B (B2 + I-REP5)** : la réputation par-variante, depuis l'attribution exposée
        // par le consensus (`won = selected_index`, `verified = score ≥ barre`). On ne crédite QUE
        // pour un run RÉUSSI — **symétrique de l'exemplaire** (`failed` plus haut) : un run dégénéré
        // (gagnant nul ⇒ tous les candidats « identiques », donc tous « vérifiés » et
        // `generator_000` « gagne ») FABRIQUERAIT des victoires/vérifications, ce qu'I-REP5 interdit,
        // et empoisonnerait le routage. `failed` ⇒ aucune attribution. Vide hors consensus aussi.
        agents: if failed {
            Vec::new()
        } else {
            contributions
                .iter()
                .map(|c| efe_swarm_memory::AgentStat { agent_id: c.agent_id.clone(), verified: c.verified, won: c.won })
                .collect()
        },
        failed,
        failure_key: Some(family.as_str().to_string()),
    }
}

/// Aperçu **déterministe** d'une valeur (l'exemplaire — jamais une valeur brute hors record).
/// Tronqué à `PREVIEW_MAX` caractères : assez long pour servir d'**exemple few-shot utile**
/// (MEM-2.b — l'aperçu EST ce qui est injecté dans le prompt Mode C), mais borné pour garder la
/// mémoire compacte et l'exposition (vue `/memory`) raisonnable.
fn preview_of(v: &Value) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.chars().count() > PREVIEW_MAX {
        s.chars().take(PREVIEW_MAX - 1).collect::<String>() + "…"
    } else {
        s
    }
}

/// Longueur max d'un aperçu d'exemplaire (cf. [`preview_of`]). Compromis few-shot/compacité.
const PREVIEW_MAX: usize = 240;

/// **ADR-019 (MEM-2.b)** : nombre d'exemplaires few-shot recall-és injectés dans le prompt d'un
/// générateur Mode C. Borne l'influence de la mémoire (I-MEM4 : amorcer, jamais gouverner) et le
/// coût en tokens. Les meilleurs (qualité-pondérés) d'abord, via [`Exemplars::top`].
pub(crate) const FEWSHOT_N: usize = 3;

/// Les aperçus des **meilleurs** exemplaires d'un instantané — ce que le run recall-e et injecte
/// dans les prompts Mode C (few-shot, borné à [`FEWSHOT_N`]). Vide ⇒ aucune influence (I-MEM8).
/// Provient de l'instantané GELÉ : l'épingle `content_hash` (déjà ancrée dans la racine par le
/// record MemoryRead, I-MEM2) couvre ces exemplaires → l'injection est re-prouvable (R1 ré-épingle).
pub(crate) fn recalled_exemplars(snap: &MemorySnapshot) -> Vec<String> {
    snap.cells.exemplars.top(FEWSHOT_N).iter().map(|e| e.preview.clone()).collect()
}
