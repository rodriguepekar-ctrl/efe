// SPDX-License-Identifier: UNLICENSED
//! **Sprint S9 R2** — Agrégateur de métriques multi-runs + générateur de
//! rapport markdown / JSON pour démontrer empirically les savings du
//! pipeline EFE sur une suite de fixtures.
//!
//! ## Vue d'ensemble
//!
//! Un test live ([`tests/live_chewing_savings_report.rs`]) orchestre N
//! runs de missions sur K fixtures × M modes (baseline/S4/S6/full/speculative).
//! Chaque run produit un [`RunRecord`] qu'on ajoute au [`SavingsReport`].
//! En fin de suite, [`SavingsReport::aggregate`] calcule les sommaires par
//! mode (cost moyen, latency moyenne, taux de qualité) et
//! [`SavingsReport::to_markdown`] produit le livrable `CHEWING_SAVINGS_REPORT.md`.
//!
//! ## Sémantique des modes
//!
//! - [`RunMode::Baseline`] : Claude direct, 0 SLM, 0 router → **coût référence**
//! - [`RunMode::S4Chewing`] : pré-mâchage Mission A activé, Claude forcé
//! - [`RunMode::S6Routing`] : table de scoring activée, pas de SLM
//! - [`RunMode::Full`] : S4 + S6 combinés (Mission A pleine)
//! - [`RunMode::Speculative`] : Mission Speculative drafter cheap + verifier premium
//!
//! ## Stabilité serde
//!
//! Tous les types sont `Serialize + Deserialize` avec format JSON stable.
//! `RunMode` utilise `SCREAMING_SNAKE_CASE` (cohérence avec `TaskClass`,
//! `FailureMode`, etc.). Le `version` field horodaté permet aux consumers
//! (CI, dashboards S15, A/B test S12) de tracer la table effective utilisée.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::ab_test::ABArm;
use crate::failure::FailureMode;
use crate::metrics::ChewingMetrics;

// ─────────────────────────────────────────────────────────────────────────
// RunMode — les 5 modes comparés dans le report
// ─────────────────────────────────────────────────────────────────────────

/// Mode d'exécution d'une mission pour la comparaison savings.
///
/// 5 variants correspondent aux 5 configurations testées par S9 :
/// `Baseline` (référence) vs 4 modes optimisés.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunMode {
    /// **Référence** : Claude direct, 0 SLM, 0 router. Définit le baseline
    /// cost et latency contre lequel toutes les optimisations sont mesurées.
    Baseline,
    /// **S4 only** : analyzer + compressor + verifier wired (Mission A
    /// full SLM), mais Claude forcé sans routing. Démontre l'impact pur
    /// du pré-mâchage sur les input tokens.
    S4Chewing,
    /// **S6 only** : table de scoring `default_3llm_table` activée, mais
    /// PAS de SLM. Démontre l'impact pur du routing.
    S6Routing,
    /// **Full** : S4 + S6 combinés. Mission A avec pré-mâchage SLM + table
    /// de scoring. Le mode "production-ready" Mission A.
    Full,
    /// **Speculative** : Mission Speculative drafter cheap (Gemini Flash) et
    /// verifier premium (Claude Sonnet 4.6). Limité aux task_class pertinentes
    /// (FactualQa principalement).
    //
    // (Le « + » ouvrait la ligne : markdown le lisait comme une PUCE et la doc
    //  rendue affichait une liste au milieu d'une phrase. Même défaut que dans
    //  `efe-swarm-provenance::cosign`. C'est le rendu qui était faux.)
    Speculative,
}

impl RunMode {
    /// Liste exhaustive des 5 variants (pour iteration tests / report).
    #[must_use]
    pub const fn all() -> &'static [RunMode] {
        &[
            RunMode::Baseline,
            RunMode::S4Chewing,
            RunMode::S6Routing,
            RunMode::Full,
            RunMode::Speculative,
        ]
    }

    /// Nom court human-friendly pour le markdown (e.g. `"baseline"`).
    #[must_use]
    pub const fn short_name(self) -> &'static str {
        match self {
            RunMode::Baseline => "baseline",
            RunMode::S4Chewing => "s4_chewing",
            RunMode::S6Routing => "s6_routing",
            RunMode::Full => "full",
            RunMode::Speculative => "speculative",
        }
    }

    /// Description longue pour les headings markdown.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            RunMode::Baseline => "Baseline (Claude direct, 0 SLM, 0 router)",
            RunMode::S4Chewing => "S4 only (pré-mâchage Mission A)",
            RunMode::S6Routing => "S6 only (routing tuné par task_class)",
            RunMode::Full => "Full (S4 + S6 combinés)",
            RunMode::Speculative => "Speculative S7 (drafter cheap + verifier premium)",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// RunRecord — 1 ligne par run de mission
// ─────────────────────────────────────────────────────────────────────────

/// Capture d'une exécution unique de mission, indexée par fixture + mode.
///
/// Construit par le test orchestrateur après chaque
/// [`run_mission_a`](crate::workflows::run_mission_a) ou
/// [`run_speculative`](crate::workflows::run_speculative).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    /// Nom de la fixture (e.g. `"01_factual_simple"`).
    pub fixture_name: String,
    /// Mode d'exécution (cf. [`RunMode`]).
    pub mode: RunMode,
    /// Coût observé en cents.
    ///
    /// **LEGACY** : utiliser `cost_microcents` (R3.S1) pour la précision sub-cent.
    pub cost_cents: u32,
    /// **R3.S1** : coût observé en microcents (1 µ¢ = 1/1000 cent).
    ///
    /// `#[serde(default)]` pour rétrocompat avec les `RunRecord` sérialisés
    /// pré-R3 (champ absent → 0 → l'agrégation utilisera `cost_cents × 1000`
    /// comme fallback via `cost_microcents_resolved()`).
    #[serde(default)]
    pub cost_microcents: u64,
    /// Latence end-to-end en ms.
    pub latency_ms: u64,
    /// Tokens output générés par le LLM final (depuis `LlmResponse.tokens_generated`).
    pub tokens_output: u32,
    /// Texte final produit (peut être tronqué pour l'audit chain).
    pub final_text_excerpt: String,
    /// LLM réellement appelé (post-fallback si applicable).
    pub llm_used: String,
    /// TaskClass de la fixture.
    pub task_class: String,
    /// Métriques détaillées (pré-mâchage, latence par phase, etc.).
    pub chewing_metrics: Option<ChewingMetrics>,
    /// La réponse contient-elle le `expected_keyword` de la fixture (lowercase) ?
    pub passed_quality: bool,
    /// Mode de failure observé (S8).
    pub failure_mode: FailureMode,
}

/// Les faits d'un run, hors coût — le coût est porté par le CONSTRUCTEUR, parce
/// que c'est lui qui en dit l'unité.
///
/// # Onze arguments, et un `allow` sans raison sur le jumeau vivant
///
/// `RunRecord::new` et `RunRecord::new_with_microcents` prenaient les mêmes onze
/// arguments positionnels, dont `llm_used` et `task_class` — deux `String`
/// **voisines**, transposables sans que le compilateur ne bronche. Aux dix-huit
/// sites d'appel, plusieurs annotaient `// µ¢` et `// ms` en fin de ligne : le
/// commentaire compensait ce que la signature ne disait pas.
///
/// É3 avait vu le défaut et ne l'avait corrigé sur aucun des deux, pour une raison
/// qui tenait : `new` était **mort** (zéro appelant), et `new_with_microcents`
/// portait déjà `#[allow(clippy::too_many_arguments)]` — posé sans une ligne de
/// justification, donc invisible à la porte de qualité. Refondre le mort en
/// laissant le vivant silencieux aurait déplacé le problème au lieu de le régler.
///
/// C'est fait ici, sur les DEUX, en un seul geste. L'`allow` disparaît avec la
/// cause plutôt que d'être documenté : c'est toujours préférable.
///
/// Le coût reste hors de cette structure, et c'est délibéré. Il est la SEULE chose
/// qui distingue les deux constructeurs ; le laisser en argument nommé par la
/// fonction (`new` = cents, `new_with_microcents` = µ¢) met l'unité sous les yeux
/// du lecteur au moment où il l'écrit. Le mettre dans la structure la cacherait.
pub struct RunFacts<'a> {
    /// Nom de la fixture jouée.
    pub fixture_name: String,
    /// Le mode dans lequel elle a tourné.
    pub mode: RunMode,
    /// Latence bout-en-bout, en millisecondes.
    pub latency_ms: u64,
    /// Tokens générés.
    pub tokens_output: u32,
    /// Le texte final — tronqué à 200 caractères à la construction.
    pub final_text: &'a str,
    /// Le LLM effectivement employé.
    pub llm_used: String,
    /// La classe de tâche retenue.
    pub task_class: String,
    /// Les métriques de pré-mâchage, si la mission en a produit.
    pub chewing_metrics: Option<ChewingMetrics>,
    /// La sortie a-t-elle passé le contrôle qualité ?
    pub passed_quality: bool,
    /// Le mode d'échec, s'il y en a un.
    pub failure_mode: FailureMode,
}

impl RunRecord {
    /// Tronc le texte à `max_chars` (utile pour l'audit chain — pas besoin
    /// de stocker le texte intégral dans le JSON du report).
    fn truncate_text(text: &str, max_chars: usize) -> String {
        if text.chars().count() <= max_chars {
            text.to_string()
        } else {
            let truncated: String = text.chars().take(max_chars).collect();
            format!("{truncated}…")
        }
    }

    /// Constructeur depuis champs bruts. Utilisé par le test live.
    ///
    /// `final_text_excerpt` est automatiquement tronqué à 200 chars.
    ///
    /// **R3.S1** : `cost_microcents` est dérivé de `cost_cents × 1000` par
    /// défaut.  Utiliser `new_with_microcents()` pour précision sub-cent.
    ///
    /// Le coût et les faits sont séparés : voir [`RunFacts`], qui porte aussi la
    /// raison du regroupement.
    ///
    /// Ce constructeur n'a **aucun appelant** — les dix-huit sites du dépôt passent
    /// tous par `new_with_microcents`, que la ligne ci-dessus désigne. Il est gardé
    /// parce qu'il est publié, et il partage désormais la même forme que son
    /// successeur : refondre l'un sans l'autre aurait laissé deux API divergentes
    /// pour un même objet.
    #[must_use]
    pub fn new(cost_cents: u32, faits: RunFacts<'_>) -> Self {
        let RunFacts {
            fixture_name,
            mode,
            latency_ms,
            tokens_output,
            final_text,
            llm_used,
            task_class,
            chewing_metrics,
            passed_quality,
            failure_mode,
        } = faits;
        Self {
            fixture_name,
            mode,
            cost_cents,
            // R3.S1 : dérivé par défaut. Utiliser `new_with_microcents` pour
            // un coût µ¢ explicite (test live R3+).
            cost_microcents: (cost_cents as u64) * 1000,
            latency_ms,
            tokens_output,
            final_text_excerpt: Self::truncate_text(final_text, 200),
            llm_used,
            task_class,
            chewing_metrics,
            passed_quality,
            failure_mode,
        }
    }

    /// **R3.S1** — Constructeur avec coût µ¢ explicite.
    ///
    /// `cost_cents` est automatiquement dérivé de `cost_microcents` via
    /// `microcents_to_cents` (policy ≥ 1 si > 0 µ¢) pour cohérence affichage.
    #[must_use]
    pub fn new_with_microcents(cost_microcents: u64, faits: RunFacts<'_>) -> Self {
        let RunFacts {
            fixture_name,
            mode,
            latency_ms,
            tokens_output,
            final_text,
            llm_used,
            task_class,
            chewing_metrics,
            passed_quality,
            failure_mode,
        } = faits;
        let cost_cents = crate::llm::microcents_to_cents(cost_microcents);
        Self {
            fixture_name,
            mode,
            cost_cents,
            cost_microcents,
            latency_ms,
            tokens_output,
            final_text_excerpt: Self::truncate_text(final_text, 200),
            llm_used,
            task_class,
            chewing_metrics,
            passed_quality,
            failure_mode,
        }
    }

    /// **R3.S1** — Coût µ¢ avec fallback vers `cost_cents × 1000` si absent.
    #[must_use]
    pub fn cost_microcents_resolved(&self) -> u64 {
        if self.cost_microcents > 0 {
            self.cost_microcents
        } else {
            (self.cost_cents as u64) * 1000
        }
    }

    /// **R3.S8** — Bras A/B de ce run (lu depuis `chewing_metrics.ab_arm`).
    /// `None` si la mission n'était pas sous A/B test.
    #[must_use]
    pub fn ab_arm(&self) -> Option<ABArm> {
        self.chewing_metrics.as_ref().and_then(|cm| cm.ab_arm)
    }

    /// **R3.S8** — Version de la table du bras A/B (traçabilité calibration).
    #[must_use]
    pub fn ab_table_version(&self) -> Option<String> {
        self.chewing_metrics
            .as_ref()
            .and_then(|cm| cm.ab_table_version.clone())
    }
}

// ─────────────────────────────────────────────────────────────────────────
// ModeSummary — agrégat par mode
// ─────────────────────────────────────────────────────────────────────────

/// Sommaire agrégé sur tous les runs d'un mode donné.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeSummary {
    /// Mode résumé.
    pub mode: RunMode,
    /// Nombre de runs comptabilisés.
    pub n_runs: usize,
    /// Somme des coûts en cents (utile pour comparer baseline_total vs full_total).
    pub total_cost_cents: u32,
    /// **R3.S1** : somme des coûts en microcents (1 µ¢ = 1/1000 cent).
    ///
    /// Précision conservée : 100 runs × 37 µ¢ = 3 700 µ¢ (= 3.7 ¢), distinct
    /// de 100 × 1 ¢ arrondi = 100 ¢ — l'erreur est 27× sur 100 runs en cents.
    /// `#[serde(default)]` pour rétrocompat.
    #[serde(default)]
    pub total_cost_microcents: u64,
    /// Coût moyen par run (cents).
    pub avg_cost_cents: f32,
    /// **R3.S1** : coût moyen par run en microcents (float pour précision).
    #[serde(default)]
    pub avg_cost_microcents: f64,
    /// Latence moyenne par run (ms).
    pub avg_latency_ms: f32,
    /// Tokens output moyens par run.
    pub avg_tokens_output: f32,
    /// Taux de runs ayant passé la qualité ∈ [0,1].
    pub quality_pass_rate: f32,
    /// Nombre total de dégradations observées (somme `degradations_count`).
    pub degradations_count: u32,
}

impl ModeSummary {
    /// Calcule le sommaire depuis une slice de records.
    /// Si `records` vide → tous les champs à 0/0.0.
    #[must_use]
    pub fn from_records(mode: RunMode, records: &[&RunRecord]) -> Self {
        let n = records.len();
        if n == 0 {
            return Self {
                mode,
                n_runs: 0,
                total_cost_cents: 0,
                total_cost_microcents: 0,
                avg_cost_cents: 0.0,
                avg_cost_microcents: 0.0,
                avg_latency_ms: 0.0,
                avg_tokens_output: 0.0,
                quality_pass_rate: 0.0,
                degradations_count: 0,
            };
        }
        let total_cost: u32 = records.iter().map(|r| r.cost_cents).sum();
        // R3.S1 : agrégation µ¢ avec saturating_add — résolu pour pré-R3.
        let total_cost_mc: u64 = records
            .iter()
            .fold(0_u64, |acc, r| acc.saturating_add(r.cost_microcents_resolved()));
        let total_lat: u64 = records.iter().map(|r| r.latency_ms).sum();
        let total_tokens: u32 = records.iter().map(|r| r.tokens_output).sum();
        let n_passed: usize = records.iter().filter(|r| r.passed_quality).count();
        let degradations: u32 = records
            .iter()
            .filter_map(|r| r.chewing_metrics.as_ref().map(|m| m.degradations_count))
            .sum();
        Self {
            mode,
            n_runs: n,
            total_cost_cents: total_cost,
            total_cost_microcents: total_cost_mc,
            avg_cost_cents: total_cost as f32 / n as f32,
            // R3.S1 : avg_µ¢ en f64 (les arrondis float ne dépassent pas 1 µ¢
            // ce qui est largement sous la précision business).
            avg_cost_microcents: total_cost_mc as f64 / n as f64,
            avg_latency_ms: total_lat as f32 / n as f32,
            avg_tokens_output: total_tokens as f32 / n as f32,
            quality_pass_rate: n_passed as f32 / n as f32,
            degradations_count: degradations,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// SavingsReport — le livrable principal S9
// ─────────────────────────────────────────────────────────────────────────

/// Rapport complet de mesure des savings sur N runs × M modes.
///
/// Workflow :
/// 1. `let mut report = SavingsReport::new();`
/// 2. Pour chaque run : `report.add_run(record);`
/// 3. En fin de suite : `report.aggregate();`
/// 4. Output : `report.to_markdown()` + `report.to_json()`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavingsReport {
    /// Tous les records bruts (une ligne par exécution).
    pub runs: Vec<RunRecord>,
    /// Sommaires par mode (rempli par `aggregate()`).
    pub summaries: HashMap<RunMode, ModeSummary>,
    /// Total cost cents pour `Baseline` (calculé par aggregate).
    pub baseline_total_cost_cents: u32,
    /// Total cost cents pour `Full` (calculé par aggregate).
    pub full_total_cost_cents: u32,
    /// **R3.S1** : total µ¢ baseline (précision sub-cent).
    #[serde(default)]
    pub baseline_total_cost_microcents: u64,
    /// **R3.S1** : total µ¢ full (précision sub-cent).
    #[serde(default)]
    pub full_total_cost_microcents: u64,
    /// Savings absolus en cents (= baseline - full). Peut être négatif si full > baseline.
    pub savings_cents: i32,
    /// **R3.S1** : savings en µ¢ (i64 pour gérer les valeurs négatives).
    #[serde(default)]
    pub savings_microcents: i64,
    /// Savings ratio ∈ [0,1]. `0.75` = 75 % savings. Peut être négatif.
    ///
    /// **LEGACY** : calculé depuis `*_cents` (peut être 0.0 quand
    /// fixtures < 1¢ chacune). Le `savings_ratio_microcents` (R3.S1) est
    /// numériquement plus stable.
    pub savings_ratio: f32,
    /// **R3.S1** : ratio savings calculé en µ¢ (= savings_µ¢ / baseline_µ¢).
    /// Numériquement stable même quand toutes les fixtures coûtent < 1 ¢.
    #[serde(default)]
    pub savings_ratio_microcents: f32,
    /// Timestamp ISO 8601 UTC de génération.
    pub generated_at: String,
    /// Version `"s9-r2-YYYY-MM-DD"`.
    pub version: String,
}

impl Default for SavingsReport {
    fn default() -> Self {
        Self::new()
    }
}

impl SavingsReport {
    /// Construit un rapport vide avec timestamp actuel + version par défaut.
    #[must_use]
    pub fn new() -> Self {
        let now = chrono::Utc::now();
        Self {
            runs: Vec::new(),
            summaries: HashMap::new(),
            baseline_total_cost_cents: 0,
            full_total_cost_cents: 0,
            baseline_total_cost_microcents: 0,
            full_total_cost_microcents: 0,
            savings_cents: 0,
            savings_microcents: 0,
            savings_ratio: 0.0,
            savings_ratio_microcents: 0.0,
            generated_at: now.to_rfc3339(),
            version: format!("r3s1-{}", now.format("%Y-%m-%d")),
        }
    }

    /// Constructeur avec version explicite (utile pour tests déterministes).
    #[must_use]
    pub fn with_version(version: impl Into<String>) -> Self {
        let mut s = Self::new();
        s.version = version.into();
        s
    }

    /// Ajoute un record. Ne déclenche PAS `aggregate` (appeler manuellement
    /// après la dernière insertion).
    pub fn add_run(&mut self, record: RunRecord) {
        self.runs.push(record);
    }

    /// Recalcule `summaries` + `baseline_*` + `full_*` + `savings_*`.
    /// Idempotent (peut être appelé plusieurs fois).
    pub fn aggregate(&mut self) {
        // Group runs by mode.
        let mut by_mode: HashMap<RunMode, Vec<&RunRecord>> = HashMap::new();
        for run in &self.runs {
            by_mode.entry(run.mode).or_default().push(run);
        }
        // Compute summary per mode.
        self.summaries.clear();
        for mode in RunMode::all() {
            let records = by_mode.get(mode).cloned().unwrap_or_default();
            self.summaries
                .insert(*mode, ModeSummary::from_records(*mode, &records));
        }
        // Calc savings baseline → full.
        self.baseline_total_cost_cents = self
            .summaries
            .get(&RunMode::Baseline)
            .map(|s| s.total_cost_cents)
            .unwrap_or(0);
        self.full_total_cost_cents = self
            .summaries
            .get(&RunMode::Full)
            .map(|s| s.total_cost_cents)
            .unwrap_or(0);
        self.savings_cents =
            self.baseline_total_cost_cents as i32 - self.full_total_cost_cents as i32;
        self.savings_ratio = if self.baseline_total_cost_cents > 0 {
            self.savings_cents as f32 / self.baseline_total_cost_cents as f32
        } else {
            0.0
        };
        // R3.S1 : agrégation µ¢ stable (gère sub-cent fidèlement).
        self.baseline_total_cost_microcents = self
            .summaries
            .get(&RunMode::Baseline)
            .map(|s| s.total_cost_microcents)
            .unwrap_or(0);
        self.full_total_cost_microcents = self
            .summaries
            .get(&RunMode::Full)
            .map(|s| s.total_cost_microcents)
            .unwrap_or(0);
        self.savings_microcents = self.baseline_total_cost_microcents as i64
            - self.full_total_cost_microcents as i64;
        self.savings_ratio_microcents = if self.baseline_total_cost_microcents > 0 {
            self.savings_microcents as f32
                / self.baseline_total_cost_microcents as f32
        } else {
            0.0
        };
    }

    /// Génère le markdown human-readable.
    /// Format stable : table résumé + section par mode + verdict.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str("# EFE Phase 3 R2 — Chewing Savings Report\n\n");
        md.push_str(&format!("> **Version** : `{}`  \n", self.version));
        md.push_str(&format!("> **Généré le** : {}\n\n", self.generated_at));
        md.push_str("---\n\n");

        // ── Résumé exécutif ──
        md.push_str("## Résumé exécutif\n\n");
        md.push_str("| Métrique | Baseline | Full (S4+S6) | Savings |\n");
        md.push_str("|---|---|---|---|\n");
        let baseline = self.summaries.get(&RunMode::Baseline);
        let full = self.summaries.get(&RunMode::Full);
        let pct_savings = (self.savings_ratio * 100.0).round() as i32;
        md.push_str(&format!(
            "| Coût total ({} fixtures) | {} ¢ | {} ¢ | **{:+} %** |\n",
            baseline.map(|s| s.n_runs).unwrap_or(0),
            self.baseline_total_cost_cents,
            self.full_total_cost_cents,
            -pct_savings,
        ));
        md.push_str(&format!(
            "| Latence moyenne | {:.0} ms | {:.0} ms | — |\n",
            baseline.map(|s| s.avg_latency_ms).unwrap_or(0.0),
            full.map(|s| s.avg_latency_ms).unwrap_or(0.0),
        ));
        md.push_str(&format!(
            "| Qualité (pass rate) | {:.0} % | {:.0} % | — |\n",
            baseline.map(|s| s.quality_pass_rate * 100.0).unwrap_or(0.0),
            full.map(|s| s.quality_pass_rate * 100.0).unwrap_or(0.0),
        ));
        md.push_str(&format!(
            "| Failure modes observés | {} | {} | — |\n",
            baseline.map(|s| s.degradations_count).unwrap_or(0),
            full.map(|s| s.degradations_count).unwrap_or(0),
        ));
        if self.baseline_total_cost_cents > 0 && self.full_total_cost_cents > 0 {
            let multiplier = self.baseline_total_cost_cents as f32
                / self.full_total_cost_cents as f32;
            md.push_str(&format!(
                "\n→ **EFE coûte ~{multiplier:.1}× moins cher que Claude direct** sur cette mixture.\n\n"
            ));
        }

        // S9 patch chirurgical : sur fixtures courtes, la résolution u32 cents
        // peut masquer les savings cost (tous arrondis à 1¢). On affiche les
        // gains **latence** + **qualité** comme axes complémentaires.
        if let (Some(baseline), Some(s6), Some(full)) = (
            baseline,
            self.summaries.get(&RunMode::S6Routing),
            self.summaries.get(&RunMode::Full),
        ) {
            md.push_str("### Savings observés sur 3 axes (résolution u32 cents limitée)\n\n");
            md.push_str("| Axe | Baseline | Optimisé | Amélioration |\n");
            md.push_str("|---|---|---|---|\n");
            // Cost
            md.push_str(&format!(
                "| **Coût** total | {} ¢ | {} ¢ (Full) | {} % |\n",
                self.baseline_total_cost_cents,
                self.full_total_cost_cents,
                (self.savings_ratio * 100.0).round() as i32,
            ));
            // Latency S6
            let s6_lat_gain = if baseline.avg_latency_ms > 0.0 {
                ((1.0 - s6.avg_latency_ms / baseline.avg_latency_ms) * 100.0).round() as i32
            } else {
                0
            };
            md.push_str(&format!(
                "| **Latence** moyenne | {:.0} ms | {:.0} ms (S6) | -{} % |\n",
                baseline.avg_latency_ms, s6.avg_latency_ms, s6_lat_gain,
            ));
            // Speculative latency if applicable
            if let Some(spec) = self.summaries.get(&RunMode::Speculative) {
                if spec.n_runs > 0 {
                    let spec_lat_gain = if baseline.avg_latency_ms > 0.0 {
                        ((1.0 - spec.avg_latency_ms / baseline.avg_latency_ms) * 100.0).round()
                            as i32
                    } else {
                        0
                    };
                    md.push_str(&format!(
                        "| **Latence** Speculative (FactualQa) | {:.0} ms | {:.0} ms | **-{} %** |\n",
                        baseline.avg_latency_ms, spec.avg_latency_ms, spec_lat_gain,
                    ));
                }
            }
            // Quality
            md.push_str(&format!(
                "| **Qualité** pass rate | {:.0} % | {:.0} % (Full) | {:+.0} pp |\n",
                baseline.quality_pass_rate * 100.0,
                full.quality_pass_rate * 100.0,
                (full.quality_pass_rate - baseline.quality_pass_rate) * 100.0,
            ));
            md.push('\n');
        }
        md.push_str("---\n\n");

        // ── Détail par mode ──
        md.push_str("## Détail par mode\n\n");
        for mode in RunMode::all() {
            let summary = self.summaries.get(mode);
            md.push_str(&format!("### {}\n\n", mode.description()));
            if let Some(s) = summary {
                if s.n_runs == 0 {
                    md.push_str("_Aucun run pour ce mode (skip)._\n\n");
                    continue;
                }
                md.push_str("| Fixture | Cost (¢) | Latency (ms) | Tokens out | LLM | Quality |\n");
                md.push_str("|---|---|---|---|---|---|\n");
                for run in self.runs.iter().filter(|r| r.mode == *mode) {
                    let q = if run.passed_quality { "✅" } else { "❌" };
                    md.push_str(&format!(
                        "| {} | {} | {} | {} | {} | {} |\n",
                        run.fixture_name,
                        run.cost_cents,
                        run.latency_ms,
                        run.tokens_output,
                        run.llm_used,
                        q,
                    ));
                }
                md.push_str(&format!(
                    "| **TOTAL** | **{} ¢** | — | — | — | — |\n",
                    s.total_cost_cents
                ));
                md.push_str(&format!(
                    "| **MOYENNE** | {:.1} ¢ | {:.0} ms | {:.0} | — | {:.0} % |\n\n",
                    s.avg_cost_cents,
                    s.avg_latency_ms,
                    s.avg_tokens_output,
                    s.quality_pass_rate * 100.0,
                ));
            } else {
                md.push_str("_Pas de runs pour ce mode._\n\n");
            }
        }
        md.push_str("---\n\n");

        // ── Verdict ──
        md.push_str("## Verdict\n\n");
        md.push_str(&format!(
            "- Cost baseline (5 fixtures) : **{} ¢**\n",
            self.baseline_total_cost_cents
        ));
        md.push_str(&format!(
            "- Cost full (5 fixtures)     : **{} ¢**\n",
            self.full_total_cost_cents
        ));
        md.push_str(&format!(
            "- **Savings**                : **{} ¢** = **{} %**\n",
            self.savings_cents, pct_savings,
        ));
        let baseline_q = baseline.map(|s| s.quality_pass_rate).unwrap_or(0.0);
        let full_q = full.map(|s| s.quality_pass_rate).unwrap_or(0.0);
        md.push_str(&format!(
            "- **Qualité préservée**      : baseline {:.0} % vs full {:.0} % {}\n\n",
            baseline_q * 100.0,
            full_q * 100.0,
            if (full_q - baseline_q).abs() < 0.01 {
                "✅ identique"
            } else if full_q >= baseline_q {
                "✅ améliorée"
            } else {
                "⚠ légère dégradation"
            }
        ));
        md.push_str(&format!(
            "**Tests exécutés** : {} runs ({} modes × {} fixtures)\n\n",
            self.runs.len(),
            self.summaries.values().filter(|s| s.n_runs > 0).count(),
            baseline.map(|s| s.n_runs).unwrap_or(0)
        ));
        md.push_str("---\n\n");

        // ── Reproductibilité ──
        md.push_str("## Reproductibilité\n\n");
        md.push_str("```bash\n");
        md.push_str("export ANTHROPIC_API_KEY=… GOOGLE_AI_KEY=… OPENAI_API_KEY=…\n");
        md.push_str("./scripts/slm/start.sh  # démarre analyzer + compressor + verifier\n");
        md.push_str("EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 cargo test -p efe-coordinator \\\n");
        md.push_str("    --features all-providers --test live_chewing_savings_report \\\n");
        md.push_str("    -- --test-threads=1 --nocapture\n");
        md.push_str("```\n\n");
        md.push_str(
            "Output regénéré : `crates/efe-coordinator/CHEWING_SAVINGS_REPORT.{md,json}`\n",
        );
        md
    }

    /// Sérialise en JSON (pretty-printed pour la lisibilité du diff).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// **R3.S1** — Rapport markdown en **microcents** (précision sub-cent).
    ///
    /// Distinct de `to_markdown()` (legacy cents) sur 3 axes :
    /// 1. Toutes les valeurs cost affichées en `X.XXX ¢` (3 décimales = µ¢).
    /// 2. Le résumé exécutif utilise `savings_ratio_microcents` (stable même
    ///    quand toutes les fixtures coûtent < 1 ¢).
    /// 3. Une note explicative en tête sur la précision µ¢ vs cents.
    ///
    /// Utilisé pour le rapport R3.S1+ — `to_markdown()` reste pour les pipes
    /// de CI/CD legacy qui s'attendent au format cents.
    #[must_use]
    pub fn to_markdown_v2(&self) -> String {
        let mut md = String::new();
        md.push_str("# EFE Phase 3 R3 — Chewing Savings Report (µ¢ precision)\n\n");
        md.push_str(&format!("> **Version** : `{}`  \n", self.version));
        md.push_str(&format!("> **Généré le** : {}  \n", self.generated_at));
        md.push_str("> **Précision** : microcents (1 µ¢ = 1/1000 cent = 1/100 000 USD).\n");
        md.push_str("> Les valeurs `X.XXX ¢` agrègent fidèlement même les missions sub-cent\n");
        md.push_str("> (Gemini Flash 1k/1k tokens = 0.037 ¢ exact ↔ 1 ¢ arrondi en legacy).\n\n");
        md.push_str("---\n\n");

        // ── Résumé exécutif (µ¢ precision) ──
        md.push_str("## Résumé exécutif (µ¢ precision)\n\n");
        md.push_str("| Métrique | Baseline | Full (S4+S6) | Savings |\n");
        md.push_str("|---|---|---|---|\n");
        let baseline = self.summaries.get(&RunMode::Baseline);
        let full = self.summaries.get(&RunMode::Full);
        let pct_savings_mc = (self.savings_ratio_microcents * 100.0).round() as i32;
        let fmt_mc = |mc: u64| format!("{:.3} ¢", mc as f64 / 1000.0);
        md.push_str(&format!(
            "| Coût total ({} fixtures) | {} | {} | **{:+} %** |\n",
            baseline.map(|s| s.n_runs).unwrap_or(0),
            fmt_mc(self.baseline_total_cost_microcents),
            fmt_mc(self.full_total_cost_microcents),
            -pct_savings_mc,
        ));
        md.push_str(&format!(
            "| Coût moyen / run | {} | {} | — |\n",
            baseline
                .map(|s| format!("{:.3} ¢", s.avg_cost_microcents / 1000.0))
                .unwrap_or_else(|| "n/a".to_string()),
            full.map(|s| format!("{:.3} ¢", s.avg_cost_microcents / 1000.0))
                .unwrap_or_else(|| "n/a".to_string()),
        ));
        md.push_str(&format!(
            "| Latence moyenne | {:.0} ms | {:.0} ms | — |\n",
            baseline.map(|s| s.avg_latency_ms).unwrap_or(0.0),
            full.map(|s| s.avg_latency_ms).unwrap_or(0.0),
        ));
        md.push_str(&format!(
            "| Qualité (pass rate) | {:.0} % | {:.0} % | — |\n",
            baseline.map(|s| s.quality_pass_rate * 100.0).unwrap_or(0.0),
            full.map(|s| s.quality_pass_rate * 100.0).unwrap_or(0.0),
        ));
        md.push_str(&format!(
            "| Failure modes observés | {} | {} | — |\n",
            baseline.map(|s| s.degradations_count).unwrap_or(0),
            full.map(|s| s.degradations_count).unwrap_or(0),
        ));
        if self.baseline_total_cost_microcents > 0 && self.full_total_cost_microcents > 0 {
            let mult = self.baseline_total_cost_microcents as f64
                / self.full_total_cost_microcents as f64;
            md.push_str(&format!(
                "\n→ **EFE coûte ~{mult:.2}× moins cher que Claude direct** (calculé µ¢).\n\n"
            ));
        }
        md.push_str("---\n\n");

        // ── Comparaison legacy cents vs µ¢ R3.S1 ──
        md.push_str("## Comparaison résolution legacy (cents) vs R3.S1 (µ¢)\n\n");
        md.push_str("| Métrique | Legacy (cents) | R3.S1 (µ¢) | Δ |\n");
        md.push_str("|---|---|---|---|\n");
        md.push_str(&format!(
            "| Baseline total | {} ¢ | {} | {:+.3} ¢ |\n",
            self.baseline_total_cost_cents,
            fmt_mc(self.baseline_total_cost_microcents),
            (self.baseline_total_cost_microcents as f64 / 1000.0)
                - self.baseline_total_cost_cents as f64,
        ));
        md.push_str(&format!(
            "| Full total | {} ¢ | {} | {:+.3} ¢ |\n",
            self.full_total_cost_cents,
            fmt_mc(self.full_total_cost_microcents),
            (self.full_total_cost_microcents as f64 / 1000.0)
                - self.full_total_cost_cents as f64,
        ));
        md.push_str(&format!(
            "| Savings ratio | {:.1} % | {:.1} % | {:+.1} pp |\n",
            self.savings_ratio * 100.0,
            self.savings_ratio_microcents * 100.0,
            (self.savings_ratio_microcents - self.savings_ratio) * 100.0,
        ));
        md.push_str("\n---\n\n");

        // ── Détail par mode (µ¢) ──
        md.push_str("## Détail par mode (µ¢ precision)\n\n");
        for mode in RunMode::all() {
            md.push_str(&format!("### {}\n\n", mode.description()));
            if let Some(s) = self.summaries.get(mode) {
                if s.n_runs == 0 {
                    md.push_str("_Aucun run pour ce mode._\n\n");
                    continue;
                }
                md.push_str(&format!(
                    "- **{} runs**, total = {}, moy = {:.3} ¢/run\n",
                    s.n_runs,
                    fmt_mc(s.total_cost_microcents),
                    s.avg_cost_microcents / 1000.0,
                ));
                md.push_str(&format!(
                    "- Latence moy = {:.0} ms, tokens moy = {:.0}, qualité = {:.0} %\n",
                    s.avg_latency_ms,
                    s.avg_tokens_output,
                    s.quality_pass_rate * 100.0,
                ));
                if s.degradations_count > 0 {
                    md.push_str(&format!(
                        "- ⚠ {} dégradations observées\n",
                        s.degradations_count
                    ));
                }
                md.push('\n');
            }
        }
        md.push_str("---\n\n");

        // ── R3.S9 (dette R3.S8 §13) — Section A/B intégrée au rapport µ¢ ──
        // `ab_section_markdown()` retourne `""` si `ab_comparison()` est `None`
        // (aucun run étiqueté A/B). L'intégration est donc un **no-op strict**
        // pour tout rapport sans A/B : les outputs existants (CI, tests) restent
        // bit-identiques. Quand des runs A/B existent, la section control vs
        // experiment (coût µ¢ / qualité / latence + Δ) s'imprime avant le footer.
        let ab = self.ab_section_markdown();
        if !ab.is_empty() {
            md.push_str(&ab);
            md.push_str("---\n\n");
        }

        md.push_str(
            "_Rapport généré via `SavingsReport::to_markdown_v2()` (R3.S1+R3.S9)._\n",
        );
        md
    }
}

// ─────────────────────────────────────────────────────────────────────────
// R3.S8 — Ségrégation A/B (control vs experiment)
// ─────────────────────────────────────────────────────────────────────────

/// Stats descriptives d'un bras A/B (sous-ensemble des runs étiquetés ce bras).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArmStats {
    /// Bras concerné.
    pub arm: ABArm,
    /// Version de la table de scoring testée par ce bras (traçabilité). Vide si
    /// non tracée.
    pub table_version: String,
    /// Nombre de runs dans ce bras.
    pub n_runs: usize,
    /// Coût moyen par run en microcents (f64 pour précision).
    pub avg_cost_microcents: f64,
    /// Taux de runs ayant passé la qualité ∈ [0,1].
    pub quality_pass_rate: f32,
    /// Latence moyenne par run (ms).
    pub avg_latency_ms: f32,
}

impl ArmStats {
    /// Agrège les stats d'un bras depuis ses records. `records` vide → 0.
    #[must_use]
    pub fn from_records(arm: ABArm, records: &[&RunRecord]) -> Self {
        let n = records.len();
        // Version de table : identique pour tous les runs d'un bras → prend la
        // première tracée.
        let table_version = records
            .iter()
            .find_map(|r| r.ab_table_version())
            .unwrap_or_default();
        if n == 0 {
            return Self {
                arm,
                table_version,
                n_runs: 0,
                avg_cost_microcents: 0.0,
                quality_pass_rate: 0.0,
                avg_latency_ms: 0.0,
            };
        }
        let total_mc: u64 = records
            .iter()
            .fold(0_u64, |acc, r| acc.saturating_add(r.cost_microcents_resolved()));
        let total_lat: u64 = records.iter().map(|r| r.latency_ms).sum();
        let n_passed = records.iter().filter(|r| r.passed_quality).count();
        Self {
            arm,
            table_version,
            n_runs: n,
            avg_cost_microcents: total_mc as f64 / n as f64,
            quality_pass_rate: n_passed as f32 / n as f32,
            avg_latency_ms: total_lat as f32 / n as f32,
        }
    }
}

/// Comparaison A/B control vs experiment + deltas (experiment − control).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ABComparison {
    /// Stats du bras de contrôle.
    pub control: ArmStats,
    /// Stats du bras d'expérimentation.
    pub experiment: ArmStats,
    /// Δ coût moyen µ¢ (négatif = experiment moins cher).
    pub cost_delta_microcents: f64,
    /// Δ taux de qualité (positif = experiment meilleur).
    pub quality_delta: f32,
    /// Δ latence moyenne ms (négatif = experiment plus rapide).
    pub latency_delta_ms: f32,
}

impl SavingsReport {
    /// **R3.S8** — Ségrège les runs par bras A/B (via
    /// `chewing_metrics.ab_arm`) et calcule une comparaison control vs
    /// experiment. Retourne `None` si **aucun** run n'est étiqueté A/B.
    #[must_use]
    pub fn ab_comparison(&self) -> Option<ABComparison> {
        let control: Vec<&RunRecord> = self
            .runs
            .iter()
            .filter(|r| r.ab_arm() == Some(ABArm::Control))
            .collect();
        let experiment: Vec<&RunRecord> = self
            .runs
            .iter()
            .filter(|r| r.ab_arm() == Some(ABArm::Experiment))
            .collect();
        if control.is_empty() && experiment.is_empty() {
            return None; // aucun run A/B
        }
        let c = ArmStats::from_records(ABArm::Control, &control);
        let e = ArmStats::from_records(ABArm::Experiment, &experiment);
        let comparison = ABComparison {
            cost_delta_microcents: e.avg_cost_microcents - c.avg_cost_microcents,
            quality_delta: e.quality_pass_rate - c.quality_pass_rate,
            latency_delta_ms: e.avg_latency_ms - c.avg_latency_ms,
            control: c,
            experiment: e,
        };
        Some(comparison)
    }

    /// **R3.S8** — Section markdown de la comparaison A/B (incluse par
    /// `to_markdown_v2` en R3.S9). Vide si pas de run A/B.
    #[must_use]
    pub fn ab_section_markdown(&self) -> String {
        let Some(cmp) = self.ab_comparison() else {
            return String::new();
        };
        let mut md = String::new();
        md.push_str("## A/B test — control vs experiment (R3.S8)\n\n");
        md.push_str("| Bras | table | runs | coût moy. (µ¢) | qualité | latence moy. (ms) |\n");
        md.push_str("|---|---|---:|---:|---:|---:|\n");
        for s in [&cmp.control, &cmp.experiment] {
            let version = if s.table_version.is_empty() {
                "—"
            } else {
                &s.table_version
            };
            md.push_str(&format!(
                "| {} | {} | {} | {:.1} | {:.1}% | {:.0} |\n",
                s.arm.as_str(),
                version,
                s.n_runs,
                s.avg_cost_microcents,
                s.quality_pass_rate * 100.0,
                s.avg_latency_ms
            ));
        }
        md.push_str(&format!(
            "\n**Δ (experiment − control)** : coût {:+.1} µ¢ · qualité {:+.1} pts · latence {:+.0} ms\n\n",
            cmp.cost_delta_microcents,
            cmp.quality_delta * 100.0,
            cmp.latency_delta_ms
        ));
        md
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests offline
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(fixture: &str, mode: RunMode, cost: u32, lat: u64, q: bool) -> RunRecord {
        RunRecord::new(
            cost,
            RunFacts {
                fixture_name: fixture.to_string(),
                mode,
                latency_ms: lat,
                tokens_output: 100,
                final_text: "stub final text",
                llm_used: "claude_claude-sonnet-4-6".to_string(),
                task_class: "FactualQa".to_string(),
                chewing_metrics: None,
                passed_quality: q,
                failure_mode: FailureMode::None,
            },
        )
    }

    #[test]
    fn runmode_serializes_screaming_snake_case() {
        for (m, expected) in [
            (RunMode::Baseline, "BASELINE"),
            (RunMode::S4Chewing, "S4_CHEWING"),
            (RunMode::S6Routing, "S6_ROUTING"),
            (RunMode::Full, "FULL"),
            (RunMode::Speculative, "SPECULATIVE"),
        ] {
            let s = serde_json::to_string(&m).unwrap();
            assert_eq!(s, format!("\"{expected}\""), "RunMode {m:?} → {expected}");
        }
    }

    #[test]
    fn run_record_roundtrips_via_json() {
        let r = make_record("01_factual", RunMode::Baseline, 5, 1500, true);
        let s = serde_json::to_string(&r).unwrap();
        let back: RunRecord = serde_json::from_str(&s).unwrap();
        assert_eq!(back.fixture_name, r.fixture_name);
        assert_eq!(back.mode, r.mode);
        assert_eq!(back.cost_cents, r.cost_cents);
    }

    #[test]
    fn aggregate_computes_avg_correctly() {
        let mut report = SavingsReport::with_version("s9-r2-test");
        report.add_run(make_record("01", RunMode::Baseline, 10, 1000, true));
        report.add_run(make_record("02", RunMode::Baseline, 20, 2000, true));
        report.add_run(make_record("03", RunMode::Baseline, 30, 3000, false));
        report.aggregate();

        let baseline = report.summaries.get(&RunMode::Baseline).unwrap();
        assert_eq!(baseline.n_runs, 3);
        assert_eq!(baseline.total_cost_cents, 60);
        assert!((baseline.avg_cost_cents - 20.0).abs() < 0.01);
        assert!((baseline.avg_latency_ms - 2000.0).abs() < 0.01);
        // 2/3 passed → ~0.667
        assert!((baseline.quality_pass_rate - 0.6667).abs() < 0.001);
    }

    #[test]
    fn aggregate_handles_empty_runs() {
        let mut report = SavingsReport::with_version("s9-r2-test");
        report.aggregate();
        for m in RunMode::all() {
            let s = report.summaries.get(m).unwrap();
            assert_eq!(s.n_runs, 0);
            assert_eq!(s.total_cost_cents, 0);
        }
        assert_eq!(report.savings_cents, 0);
        assert_eq!(report.savings_ratio, 0.0);
    }

    #[test]
    fn aggregate_computes_savings_baseline_vs_full() {
        let mut report = SavingsReport::with_version("s9-r2-test");
        // Baseline : 5 fixtures × 5¢ = 25¢
        for i in 0..5 {
            report.add_run(make_record(&format!("f{i}"), RunMode::Baseline, 5, 1000, true));
        }
        // Full : 5 fixtures × 1¢ = 5¢ → savings 20¢ = 80%
        for i in 0..5 {
            report.add_run(make_record(&format!("f{i}"), RunMode::Full, 1, 800, true));
        }
        report.aggregate();
        assert_eq!(report.baseline_total_cost_cents, 25);
        assert_eq!(report.full_total_cost_cents, 5);
        assert_eq!(report.savings_cents, 20);
        assert!(
            (report.savings_ratio - 0.8).abs() < 0.001,
            "savings_ratio ≈ 0.8 attendu, got {}",
            report.savings_ratio
        );
    }

    #[test]
    fn to_markdown_includes_summary_table() {
        let mut report = SavingsReport::with_version("s9-r2-test");
        report.add_run(make_record("01", RunMode::Baseline, 5, 1000, true));
        report.add_run(make_record("01", RunMode::Full, 1, 800, true));
        report.aggregate();
        let md = report.to_markdown();
        assert!(md.contains("# EFE Phase 3 R2 — Chewing Savings Report"));
        assert!(md.contains("Résumé exécutif"));
        assert!(md.contains("Baseline"));
        assert!(md.contains("Full"));
        assert!(md.contains("Verdict"));
        assert!(md.contains("Reproductibilité"));
        // Doit contenir la commande de relance.
        assert!(md.contains("EFE_LIVE_LLM=1"));
    }

    #[test]
    fn to_markdown_contains_one_section_per_mode() {
        let mut report = SavingsReport::with_version("s9-r2-test");
        report.add_run(make_record("01", RunMode::Baseline, 5, 1000, true));
        report.aggregate();
        let md = report.to_markdown();
        // Chaque mode doit avoir sa section description (même vide).
        for mode in RunMode::all() {
            assert!(
                md.contains(mode.description()),
                "markdown doit contenir la section '{}'",
                mode.description()
            );
        }
    }

    #[test]
    fn to_json_roundtrips_complete_report() {
        let mut report = SavingsReport::with_version("s9-r2-test");
        report.add_run(make_record("01", RunMode::Baseline, 5, 1000, true));
        report.add_run(make_record("01", RunMode::Full, 1, 800, true));
        report.aggregate();
        let json = report.to_json().unwrap();
        let back: SavingsReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.runs.len(), 2);
        assert_eq!(back.baseline_total_cost_cents, 5);
        assert_eq!(back.full_total_cost_cents, 1);
        assert_eq!(back.savings_cents, 4);
    }

    #[test]
    fn savings_ratio_handles_zero_baseline_safely() {
        let mut report = SavingsReport::with_version("s9-r2-test");
        // Pas de baseline, juste full → division par zéro safe.
        report.add_run(make_record("01", RunMode::Full, 1, 800, true));
        report.aggregate();
        assert_eq!(report.savings_ratio, 0.0, "0/0 doit retourner 0, pas NaN");
    }

    #[test]
    fn quality_pass_rate_aggregates_correctly() {
        let mut report = SavingsReport::with_version("s9-r2-test");
        // 3 passed, 2 failed = 60%
        report.add_run(make_record("01", RunMode::Full, 1, 800, true));
        report.add_run(make_record("02", RunMode::Full, 1, 800, true));
        report.add_run(make_record("03", RunMode::Full, 1, 800, true));
        report.add_run(make_record("04", RunMode::Full, 1, 800, false));
        report.add_run(make_record("05", RunMode::Full, 1, 800, false));
        report.aggregate();
        let full = report.summaries.get(&RunMode::Full).unwrap();
        assert!((full.quality_pass_rate - 0.6).abs() < 0.001);
    }

    #[test]
    fn version_format_is_sprint_release_yyyy_mm_dd() {
        let report = SavingsReport::new();
        // R3.S1 : version par défaut a bougé de "s9-r2-..." à "r3s1-...".
        assert!(report.version.starts_with("r3s1-"));
        // Format YYYY-MM-DD doit ajouter 10 chars après le prefix.
        assert_eq!(report.version.len(), "r3s1-".len() + 10);
    }

    #[test]
    fn mode_summary_from_empty_slice_returns_zeros() {
        let s = ModeSummary::from_records(RunMode::Full, &[]);
        assert_eq!(s.n_runs, 0);
        assert_eq!(s.total_cost_cents, 0);
        assert_eq!(s.avg_cost_cents, 0.0);
        assert_eq!(s.quality_pass_rate, 0.0);
    }

    #[test]
    fn run_record_truncates_long_final_text() {
        let long_text = "A".repeat(500);
        let r = RunRecord::new(
            1,
            RunFacts {
                fixture_name: "x".to_string(),
                mode: RunMode::Baseline,
                latency_ms: 100,
                tokens_output: 10,
                final_text: &long_text,
                llm_used: "claude".to_string(),
                task_class: "FactualQa".to_string(),
                chewing_metrics: None,
                passed_quality: true,
                failure_mode: FailureMode::None,
            },
        );
        // Doit être tronqué à 200 chars + "…" → 201 chars max.
        assert!(r.final_text_excerpt.chars().count() <= 201);
        assert!(r.final_text_excerpt.ends_with('…'));
    }

    // ============================================================
    // R3.S1 — Tests microcents
    // ============================================================

    #[test]
    fn r3s1_new_record_derives_microcents_from_cents() {
        let r = make_record("01", RunMode::Baseline, 5, 1000, true);
        assert_eq!(r.cost_cents, 5);
        // R3.S1 default : cost_microcents = cost_cents × 1000.
        assert_eq!(r.cost_microcents, 5_000);
    }

    #[test]
    fn r3s1_new_with_microcents_derives_cents() {
        // Gemini Flash 1k/1k = 37 µ¢ exact → arrondi 1 ¢.
        let r = RunRecord::new_with_microcents(
            37,
            RunFacts {
                fixture_name: "flash_test".to_string(),
                mode: RunMode::Speculative,
                latency_ms: 150,
                tokens_output: 10,
                final_text: "ok",
                llm_used: "gemini".to_string(),
                task_class: "FactualQa".to_string(),
                chewing_metrics: None,
                passed_quality: true,
                failure_mode: FailureMode::None,
            },
        );
        assert_eq!(r.cost_microcents, 37);
        assert_eq!(r.cost_cents, 1); // 37 µ¢ < 500 → arrondi à 1 (policy ≥ 1).
    }

    #[test]
    fn r3s1_aggregate_microcents_sub_cent_precision() {
        // 100 fixtures × 37 µ¢ = 3 700 µ¢ = 3.7 ¢.
        // En legacy : 100 × 1¢ = 100 ¢ (arrondi → +27× drift).
        let mut report = SavingsReport::with_version("r3s1-test");
        for i in 0..100 {
            report.add_run(RunRecord::new_with_microcents(
                37,
                RunFacts {
                    fixture_name: format!("f{i}"),
                    mode: RunMode::Speculative,
                    latency_ms: 100,
                    tokens_output: 10,
                    final_text: "ok",
                    llm_used: "g".to_string(),
                    task_class: "FactualQa".to_string(),
                    chewing_metrics: None,
                    passed_quality: true,
                    failure_mode: FailureMode::None,
                },
            ));
        }
        report.aggregate();
        let spec = report.summaries.get(&RunMode::Speculative).unwrap();
        assert_eq!(spec.total_cost_microcents, 3_700);
        // Legacy cents : 100 × 1 = 100.
        assert_eq!(spec.total_cost_cents, 100);
        // L'erreur legacy en cents est de +96.3 ¢ vs réalité 3.7 ¢ (27×).
    }

    #[test]
    fn r3s1_savings_ratio_microcents_stable_when_cents_arrondis() {
        // Sub-cent : baseline = 100 × 37 µ¢ = 3 700 µ¢, full = 100 × 7 µ¢ = 700 µ¢.
        // savings_ratio_µ¢ ≈ (3700-700)/3700 = 0.81
        // En cents : tous arrondis à 1 → savings_ratio_cents = 0.
        let mut report = SavingsReport::with_version("r3s1-test");
        for i in 0..100 {
            report.add_run(RunRecord::new_with_microcents(
                37,
                RunFacts {
                    fixture_name: format!("b{i}"),
                    mode: RunMode::Baseline,
                    latency_ms: 100,
                    tokens_output: 10,
                    final_text: "ok",
                    llm_used: "claude".into(),
                    task_class: "FactualQa".into(),
                    chewing_metrics: None,
                    passed_quality: true,
                    failure_mode: FailureMode::None,
                },
            ));
            report.add_run(RunRecord::new_with_microcents(
                7,
                RunFacts {
                    fixture_name: format!("f{i}"),
                    mode: RunMode::Full,
                    latency_ms: 100,
                    tokens_output: 10,
                    final_text: "ok",
                    llm_used: "flash".into(),
                    task_class: "FactualQa".into(),
                    chewing_metrics: None,
                    passed_quality: true,
                    failure_mode: FailureMode::None,
                },
            ));
        }
        report.aggregate();
        // savings_ratio µ¢ doit être très proche de 0.81.
        assert!(
            (report.savings_ratio_microcents - 0.81).abs() < 0.01,
            "savings_ratio µ¢ ≈ 0.81 attendu, got {}",
            report.savings_ratio_microcents
        );
        // savings_ratio legacy cents : DOIT être 0.0 (tous arrondis à 1¢ →
        // baseline 100 ¢, full 100 ¢, ratio 0).
        assert_eq!(report.savings_ratio, 0.0);
    }

    #[test]
    fn r3s1_to_markdown_v2_contains_microcents_precision() {
        let mut report = SavingsReport::with_version("r3s1-test");
        report.add_run(RunRecord::new_with_microcents(
            37_000,
            RunFacts {
                fixture_name: "01".to_string(),
                mode: RunMode::Baseline,
                latency_ms: 1000,
                tokens_output: 10,
                final_text: "ok",
                llm_used: "claude".to_string(),
                task_class: "FactualQa".to_string(),
                chewing_metrics: None,
                passed_quality: true,
                failure_mode: FailureMode::None,
            },
        ));
        report.add_run(RunRecord::new_with_microcents(
            7_000,
            RunFacts {
                fixture_name: "01".to_string(),
                mode: RunMode::Full,
                latency_ms: 800,
                tokens_output: 10,
                final_text: "ok",
                llm_used: "flash".to_string(),
                task_class: "FactualQa".to_string(),
                chewing_metrics: None,
                passed_quality: true,
                failure_mode: FailureMode::None,
            },
        ));
        report.aggregate();
        let md = report.to_markdown_v2();
        assert!(md.contains("µ¢ precision"));
        assert!(md.contains("microcents"));
        // Format X.XXX ¢ visible.
        assert!(md.contains("37.000 ¢") || md.contains("37.0 ¢"), "markdown v2 doit afficher 37.000 ¢");
    }

    #[test]
    fn r3s1_run_record_backward_compat_pre_r3_json() {
        // JSON sérialisé pré-R3 (sans cost_microcents).
        // FailureMode est adjacently tagged : `{"kind": "NONE"}`.
        let json = r#"{
            "fixture_name": "01_legacy",
            "mode": "BASELINE",
            "cost_cents": 5,
            "latency_ms": 1000,
            "tokens_output": 10,
            "final_text_excerpt": "ok",
            "llm_used": "claude",
            "task_class": "FactualQa",
            "chewing_metrics": null,
            "passed_quality": true,
            "failure_mode": {"kind": "NONE"}
        }"#;
        let r: RunRecord = serde_json::from_str(json).unwrap();
        assert_eq!(r.cost_microcents, 0); // default
        // Mais cost_microcents_resolved() doit dériver depuis cents.
        assert_eq!(r.cost_microcents_resolved(), 5_000);
    }

    // ─────────────────────────────────────────────────────────────────────
    // R3.S9 — D4 : `to_markdown_v2()` intègre la section A/B (dette R3.S8 §13)
    // ─────────────────────────────────────────────────────────────────────

    /// Construit un `RunRecord` étiqueté A/B (bras + version de table) via
    /// `chewing_metrics`. Coût µ¢ explicite pour exercer `ArmStats`.
    fn ab_record(arm: crate::ab_test::ABArm, version: &str, cost_mc: u64) -> RunRecord {
        let cm = ChewingMetrics {
            ab_arm: Some(arm),
            ab_table_version: Some(version.to_string()),
            ..ChewingMetrics::default()
        };
        RunRecord::new_with_microcents(
            cost_mc,
            RunFacts {
                fixture_name: "fixture".to_string(),
                mode: RunMode::Full,
                latency_ms: 100,
                tokens_output: 10,
                final_text: "réponse complète et non triviale",
                llm_used: "llm".to_string(),
                task_class: "FactualQa".to_string(),
                chewing_metrics: Some(cm),
                passed_quality: true,
                failure_mode: FailureMode::None,
            },
        )
    }

    #[test]
    fn r3s9_to_markdown_v2_includes_ab_section_when_present() {
        use crate::ab_test::ABArm;
        let mut report = SavingsReport::with_version("s9-r3-test");
        report.add_run(ab_record(ABArm::Control, "ctrl-v1", 3_000));
        report.add_run(ab_record(ABArm::Experiment, "exp-v2", 500));
        report.aggregate();

        let md = report.to_markdown_v2();
        // Le rapport µ¢ garde son en-tête + intègre désormais la section A/B.
        assert!(md.contains("µ¢ precision"), "header µ¢ conservé");
        assert!(
            md.contains("A/B test — control vs experiment"),
            "D4: section A/B présente quand des runs A/B existent"
        );
        assert!(md.contains("ctrl-v1"), "version control tracée");
        assert!(md.contains("exp-v2"), "version experiment tracée");
        // La section A/B doit apparaître AVANT le footer générateur.
        let ab_pos = md.find("A/B test — control").expect("section A/B");
        let footer_pos = md.find("_Rapport généré via").expect("footer");
        assert!(ab_pos < footer_pos, "D4: section A/B avant le footer");
    }

    #[test]
    fn r3s9_to_markdown_v2_omits_ab_section_when_absent() {
        // Aucun run A/B → `ab_section_markdown()` renvoie "" → no-op strict.
        let mut report = SavingsReport::with_version("s9-r3-test");
        report.add_run(RunRecord::new_with_microcents(
            3_000,
            RunFacts {
                fixture_name: "01".to_string(),
                mode: RunMode::Baseline,
                latency_ms: 1000,
                tokens_output: 10,
                final_text: "réponse baseline",
                llm_used: "claude".to_string(),
                task_class: "FactualQa".to_string(),
                chewing_metrics: None,
                passed_quality: true,
                failure_mode: FailureMode::None,
            },
        ));
        report.aggregate();

        let md = report.to_markdown_v2();
        assert!(md.contains("µ¢ precision"), "header µ¢ conservé");
        assert!(
            !md.contains("A/B test — control vs experiment"),
            "D4: aucune section A/B sans run A/B (backward-compat strict)"
        );
        assert!(
            md.contains("_Rapport généré via"),
            "footer générateur toujours présent"
        );
    }
}
