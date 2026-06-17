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
//! ## Périmètre honnête de MEM-2
//! MEM-2 capte ce qui est **fiablement exposé** par un `SwarmRun` : `k_used` (le prior de
//! taille, qui pilote le warm-start) et le **gagnant** comme exemplaire few-shot. Le **pattern
//! gagnant** (visible seulement sur le flux observateur, pas sur `SwarmRun`) et la **réputation
//! par-agent** ne sont pas encore captés — laissés vides (jamais fabriqués). Leur lecture en
//! retour (biais du sélecteur de pattern, injection d'exemplaires dans les prompts Mode C) est
//! couplée à la topologie / au Mode C : c'est l'incrément suivant (MEM-2.b / MEM-3).

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
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self { recall: true, learn: true, mode: FamilyMode::Structural, family_override: None }
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
) -> CallCapture {
    let input = json!({ "family": family.as_str(), "op": "recall" });
    let output = json!({
        "version": snap.version,
        "content_hash": snap.content_hash_hex(),
        "warm_start_k_min": warm_k_min,
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
/// **fiablement exposé** : `k_used` (le K, via `quality.candidates`) et le **gagnant** comme
/// exemplaire few-shot (content-addressed). Pattern gagnant + réputation : vides en MEM-2
/// (non exposés sur `SwarmRun` — cf. doc du module). `clean` reflète `is_clean()` ; sur le
/// chemin learn (gardé), il est toujours vrai, mais on le porte pour la traçabilité.
pub(crate) fn summary_from_run(run: &SwarmRun) -> RunSummary {
    // Le gagnant : la valeur agrégée (`output.value` pour consensus/T1 ; sinon la sortie nue).
    let winner = run.output.get("value").cloned().unwrap_or_else(|| run.output.clone());
    let exemplar = Exemplar {
        content_hash: blake3_value(&winner).to_hex(),
        preview: preview_of(&winner),
        // Un run propre : son gagnant est la réponse acceptée → pleine qualité. La graduation
        // fine (confiance d'agrégation) viendra quand `SwarmRun` l'exposera (MEM-3).
        quality_milli: 1000,
    };
    RunSummary {
        clean: run.is_clean(),
        k_used: run.quality.candidates as u32,
        winning_pattern: None,
        escalated: false,
        exemplars: vec![exemplar],
        agents: Vec::new(),
        failed: false,
        failure_key: None,
    }
}

/// Aperçu court + déterministe d'une valeur (pour l'exemplaire — jamais une valeur brute hors
/// record). Tronqué à 80 caractères.
fn preview_of(v: &Value) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.chars().count() > 80 {
        s.chars().take(77).collect::<String>() + "…"
    } else {
        s
    }
}
