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
    /// **Speculative** : Mission Speculative drafter cheap (Gemini Flash)
    /// + verifier premium (Claude Sonnet 4.6). Limité aux task_class
    /// pertinentes (FactualQa principalement).
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
    pub cost_cents: u32,
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
    #[must_use]
    pub fn new(
        fixture_name: String,
        mode: RunMode,
        cost_cents: u32,
        latency_ms: u64,
        tokens_output: u32,
        final_text: &str,
        llm_used: String,
        task_class: String,
        chewing_metrics: Option<ChewingMetrics>,
        passed_quality: bool,
        failure_mode: FailureMode,
    ) -> Self {
        Self {
            fixture_name,
            mode,
            cost_cents,
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
    /// Coût moyen par run (cents).
    pub avg_cost_cents: f32,
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
                avg_cost_cents: 0.0,
                avg_latency_ms: 0.0,
                avg_tokens_output: 0.0,
                quality_pass_rate: 0.0,
                degradations_count: 0,
            };
        }
        let total_cost: u32 = records.iter().map(|r| r.cost_cents).sum();
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
            avg_cost_cents: total_cost as f32 / n as f32,
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
    /// Savings absolus en cents (= baseline - full). Peut être négatif si full > baseline.
    pub savings_cents: i32,
    /// Savings ratio ∈ [0,1]. `0.75` = 75 % savings. Peut être négatif.
    pub savings_ratio: f32,
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
            savings_cents: 0,
            savings_ratio: 0.0,
            generated_at: now.to_rfc3339(),
            version: format!("s9-r2-{}", now.format("%Y-%m-%d")),
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
                "\n→ **EFE coûte ~{:.1}× moins cher que Claude direct** sur cette mixture.\n\n",
                multiplier
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
            md.push_str("\n");
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
}

// ─────────────────────────────────────────────────────────────────────────
// Tests offline
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(fixture: &str, mode: RunMode, cost: u32, lat: u64, q: bool) -> RunRecord {
        RunRecord::new(
            fixture.to_string(),
            mode,
            cost,
            lat,
            100, // tokens output stub
            "stub final text",
            "claude_claude-sonnet-4-6".to_string(),
            "FactualQa".to_string(),
            None,
            q,
            FailureMode::None,
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
        assert!(report.version.starts_with("s9-r2-"));
        // Format YYYY-MM-DD doit ajouter 10 chars après le prefix.
        assert_eq!(report.version.len(), "s9-r2-".len() + 10);
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
            "x".to_string(),
            RunMode::Baseline,
            1,
            100,
            10,
            &long_text,
            "claude".to_string(),
            "FactualQa".to_string(),
            None,
            true,
            FailureMode::None,
        );
        // Doit être tronqué à 200 chars + "…" → 201 chars max.
        assert!(r.final_text_excerpt.chars().count() <= 201);
        assert!(r.final_text_excerpt.ends_with('…'));
    }
}
