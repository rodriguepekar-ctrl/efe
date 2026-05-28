// SPDX-License-Identifier: UNLICENSED
//! Module `metrics` — quantification du gain de pré-mâchage (Sprint S4 R2).
//!
//! Capture les économies de tokens d'entrée du gros LLM grâce au pipeline
//! SLM (analyzer → router → compressor). Émis comme `StreamEvent::PreChewingMetrics`
//! en fin de mission et stocké dans `MissionAResponse.chewing_metrics` pour audit.
//!
//! ## Conventions
//!
//! - **Tokens** : estimés via l'heuristique `1 token ≈ 4 chars` (cohérente avec
//!   `crates/efe-nano-app-coordinator/src/clients/filter.rs:327`). Pas de
//!   dépendance tiktoken — précision suffisante pour reporting macroscopique.
//! - **Ratios** : `compression_ratio ∈ [0.0, 1.0]` où `0.25` signifie « 25 %
//!   des tokens d'origine restants » (= 75 % de savings).
//! - **Latences** : milliseconds, `u64`. Si une étape est skippée (robot
//!   absent), sa latence reste 0.
//! - **Tracabilité router** : `selected_llm` + `router_reason` permettent de
//!   débugger les choix Pareto en cas de routing inattendu.
//!
//! ## Réutilisabilité
//!
//! `ChewingMetrics` est sérialisable serde et public. Utilisable :
//! - dans `MissionAResponse.chewing_metrics: Option<ChewingMetrics>` (S4)
//! - dans `MissionBResponse` (S5, multi-LLM avec pré-mâchage mutualisé)
//! - dans `SpeculativeResponse` (S7, drafter + verifier metrics)
//! - dans `CHEWING_SAVINGS_REPORT.md` (S9, agrégation multi-fixtures)

use serde::{Deserialize, Serialize};

use crate::failure::FailureMode;

/// Métriques quantifiant le gain de pré-mâchage SLM avant l'appel au gros LLM.
///
/// Sérialisable JSON via serde. Tous les champs sont publics pour permettre
/// la lecture directe par les tests, les UIs et les outils d'audit.
///
/// **Cible R2** : `compression_ratio ≤ 0.7` (≥ 30 % de réduction de tokens
/// d'entrée), avec verdict verifier toujours `SUPPORTED`. Cette cible sera
/// agrégée sur 5 fixtures en S9.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ChewingMetrics {
    // ── Tokens (heuristique 1 token ≈ 4 chars) ────────────────────────────

    /// Tokens d'entrée bruts (avant pré-mâchage) — depuis `req.user_text`.
    pub input_tokens_raw: u32,

    /// Tokens d'entrée envoyés au gros LLM (après compressor si actif).
    /// Si compressor skippé : `= input_tokens_raw`.
    pub input_tokens_compressed: u32,

    /// Tokens générés par le gros LLM en sortie (depuis `LlmResponse.tokens_generated`).
    /// Note : cette valeur vient du provider donc elle est exacte (non heuristique).
    pub llm_tokens_generated: u32,

    // ── Ratios (sav = 1 - compression_ratio) ──────────────────────────────

    /// Ratio `compressed / raw`. ∈ `[0.0, 1.0]`. `1.0` = pas compressé.
    /// Exemple : `0.25` → 75 % de savings.
    pub compression_ratio: f32,

    /// Tokens économisés = `raw - compressed`. Toujours ≥ 0.
    pub savings_tokens: u32,

    /// Ratio des savings = `1.0 - compression_ratio`. ∈ `[0.0, 1.0]`.
    /// Exemple : `0.75` → 75 % d'économie.
    pub savings_ratio: f32,

    // ── Latences par phase (ms) ──────────────────────────────────────────

    /// Latence de l'analyzer (0 si skippé).
    pub analyzer_latency_ms: u64,

    /// Latence du router (0 si skippé). Heuristique in-process, typiquement < 10 ms.
    pub router_latency_ms: u64,

    /// Latence du compressor (0 si skippé ou si `user_text.len() ≤ 500`).
    pub compressor_latency_ms: u64,

    /// Latence de l'appel au gros LLM (depuis `LlmResponse.latency_ms`).
    pub llm_latency_ms: u64,

    /// Latence du verifier (0 si skippé ou si `candidate_sources.is_empty()`).
    pub verifier_latency_ms: u64,

    /// Somme des latences SLM (analyzer + compressor + verifier). Le router
    /// est négligeable et inclus dans `total_pipeline_latency_ms` mais pas ici
    /// (heuristique Rust, pas un LLM).
    pub total_slm_latency_ms: u64,

    /// Latence totale wall-clock de `run_mission_a`.
    pub total_pipeline_latency_ms: u64,

    // ── Choix Router + traçabilité ─────────────────────────────────────────

    /// ID du LLM sélectionné par le router (ou par fallback si absent).
    pub selected_llm: String,

    /// Justification humaine du choix (≤ 200 chars). Vide si pas de router.
    pub router_reason: String,

    /// Coût estimé en cents par le router avant l'appel (différent de
    /// `LlmResponse.cost_cents` qui est l'observé post-call).
    ///
    /// **LEGACY** : utiliser `router_estimated_cost_microcents` (R3.S1) pour
    /// la précision sub-cent (Gemini Flash).
    pub router_estimated_cost_cents: u32,

    /// **R3.S1** : coût estimé en microcents (1 µ¢ = 1/1000 cent).
    ///
    /// `#[serde(default)]` pour backward-compat avec les `ChewingMetrics`
    /// sérialisés pré-R3 (champ absent → 0).
    #[serde(default)]
    pub router_estimated_cost_microcents: u64,

    // ── Sprint S7 R2 — Métriques Mission Speculative ─────────────────────
    //
    // Ces champs sont remplis UNIQUEMENT par `run_speculative` (Mission Spec).
    // Pour Mission A/B, ils restent à `None` / `0`. Cohérent avec le pattern
    // `chewing_metrics` polymorphe : certains champs sont mission-dependent.
    // Tous `#[serde(default)]` pour backward-compat avec les JSON pré-S7.
    /// **S7** — Le verifier API a-t-il été appelé pour Mission Spec ?
    ///
    /// - `Some(true)` (HOT PATH) : drafter seul a suffi, savings ~100 % sur verifier
    /// - `Some(false)` (SLOW PATH) : verifier appelé, savings 0
    /// - `None` : Mission non-spec (A ou B), champ n'a pas de sens
    #[serde(default)]
    pub verifier_skipped: Option<bool>,

    /// **S7** — Latence du drafter spéculatif (0 si non-spec).
    #[serde(default)]
    pub spec_drafter_latency_ms: u64,

    /// **S7** — Latence du verifier spéculatif (0 si skipped OU non-spec).
    #[serde(default)]
    pub spec_verifier_latency_ms: u64,

    /// **S7** — Coût économisé estimé en cents si `verifier_skipped == true`.
    /// Calculé via `verifier_metadata.cost_*_per_million × tokens_attendus`.
    ///
    /// **LEGACY** : utiliser `spec_savings_microcents` (R3.S1).
    #[serde(default)]
    pub spec_savings_cents: u32,

    /// **R3.S1** : économies en microcents quand HOT PATH déclenche le bypass
    /// du verifier.  Précision sub-cent : un Speculative qui économise 0.5 ¢
    /// (= 500 µ¢) sur 1 mission n'apparaît PAS dans `spec_savings_cents`
    /// (arrondi à 0 ou 1) mais s'agrège honnêtement sur 1000 missions
    /// (= 500_000 µ¢ = 500 ¢ = 5 $).
    #[serde(default)]
    pub spec_savings_microcents: u64,

    /// **R3.S3 (fix L4/D6)** — Le draft du drafter a-t-il été rejeté comme
    /// bruit (forçant le slow path) ?
    ///
    /// - `Some(true)` : draft-bruit détecté (« La », refus…) → hot path
    ///   bloqué → verifier appelé → vraie réponse produite.
    /// - `Some(false)` : Speculative sans rejet de bruit (hot path nominal
    ///   OU slow path par longueur).
    /// - `None` : mission non-Speculative (A ou B).
    ///
    /// Le taux de `Some(true)` est un **signal de santé du drafter** :
    /// > 30 % ⇒ drafter mal choisi (R3.S4 le remplacera).
    #[serde(default)]
    pub spec_noise_rejected: Option<bool>,

    // ── Sprint S8 R2 — Graceful degradation tracing ───────────────────────
    /// **S8** — Mode de failure observé pendant la mission.
    ///
    /// `FailureMode::None` (default) pour cas nominal. Permet à un audit /
    /// dashboard de réagir : alerte si > 5 % missions dégradées, etc.
    #[serde(default)]
    pub failure_mode: FailureMode,

    /// **S8** — Nombre total de dégradations distinctes (= 1 pour simple,
    /// `len()` pour Composite). Pratique pour métriques agrégées (S9).
    #[serde(default)]
    pub degradations_count: u32,

    /// **S8** — Liste des composants ayant fail (`"analyzer"`,
    /// `"compressor"`, `"llm:claude_…"`). Ordre stable d'observation.
    #[serde(default)]
    pub failed_components: Vec<String>,
}

impl ChewingMetrics {
    /// Estime le nombre de tokens d'un texte via l'heuristique
    /// **1 token ≈ 4 caractères Unicode**.
    ///
    /// Aligné avec `filter.rs:327` du crate `efe-nano-app-coordinator`.
    /// Cette heuristique est suffisante pour le reporting macroscopique des
    /// économies — la précision absolue (via tiktoken ou Claude tokenizer)
    /// est hors-scope R2.
    ///
    /// ## Exemples
    ///
    /// ```
    /// use efe_coordinator::ChewingMetrics;
    /// assert_eq!(ChewingMetrics::estimate_tokens(""), 0);
    /// assert_eq!(ChewingMetrics::estimate_tokens("a"), 1);     // ceil(0.25) = 1
    /// assert_eq!(ChewingMetrics::estimate_tokens("abcd"), 1);  // ceil(1.0) = 1
    /// assert_eq!(ChewingMetrics::estimate_tokens("abcde"), 2); // ceil(1.25) = 2
    /// ```
    #[must_use]
    pub fn estimate_tokens(text: &str) -> u32 {
        if text.is_empty() {
            return 0;
        }
        ((text.chars().count() as f32 / 4.0).ceil()) as u32
    }

    /// Recalcule `compression_ratio`, `savings_tokens`, `savings_ratio` à
    /// partir de `input_tokens_raw` et `input_tokens_compressed`.
    ///
    /// À appeler **après** avoir setté ces 2 champs (sinon les ratios
    /// resteront à 0 par défaut). Idempotent : peut être appelé plusieurs
    /// fois sans effet de bord.
    ///
    /// ## Cas particuliers
    ///
    /// - Si `input_tokens_raw == 0` : `compression_ratio = 1.0` (pas de division
    ///   par zéro), `savings_tokens = 0`, `savings_ratio = 0.0`.
    /// - Si `input_tokens_compressed > input_tokens_raw` (cas pathologique
    ///   où le compressor a *augmenté* le texte) : `compression_ratio > 1.0`,
    ///   `savings_tokens = 0` (`saturating_sub`), `savings_ratio` est négatif
    ///   ce qui signale clairement le problème.
    pub fn recompute_savings(&mut self) {
        self.compression_ratio = if self.input_tokens_raw > 0 {
            self.input_tokens_compressed as f32 / self.input_tokens_raw as f32
        } else {
            1.0
        };
        self.savings_tokens = self
            .input_tokens_raw
            .saturating_sub(self.input_tokens_compressed);
        self.savings_ratio = 1.0 - self.compression_ratio;
    }

    /// Recalcule `total_slm_latency_ms` = analyzer + compressor + verifier.
    ///
    /// Router exclu (heuristique in-process Rust, pas un SLM via llama-server).
    pub fn recompute_total_slm_latency(&mut self) {
        self.total_slm_latency_ms =
            self.analyzer_latency_ms + self.compressor_latency_ms + self.verifier_latency_ms;
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_basic() {
        assert_eq!(ChewingMetrics::estimate_tokens(""), 0);
        assert_eq!(ChewingMetrics::estimate_tokens("a"), 1);
        assert_eq!(ChewingMetrics::estimate_tokens("abc"), 1);
        assert_eq!(ChewingMetrics::estimate_tokens("abcd"), 1);
        assert_eq!(ChewingMetrics::estimate_tokens("abcde"), 2);
        // 400 caractères → 100 tokens
        let text_400 = "x".repeat(400);
        assert_eq!(ChewingMetrics::estimate_tokens(&text_400), 100);
    }

    #[test]
    fn estimate_tokens_unicode_safe() {
        // "é" est 1 caractère Unicode mais 2 octets en UTF-8.
        // On compte les caractères, pas les octets.
        assert_eq!(ChewingMetrics::estimate_tokens("éééé"), 1); // 4 chars → 1 token
        assert_eq!(ChewingMetrics::estimate_tokens("é"), 1); // ceil(0.25) = 1
    }

    #[test]
    fn recompute_savings_nominal() {
        let mut m = ChewingMetrics::default();
        m.input_tokens_raw = 100;
        m.input_tokens_compressed = 25;
        m.recompute_savings();
        assert_eq!(m.compression_ratio, 0.25);
        assert_eq!(m.savings_tokens, 75);
        assert_eq!(m.savings_ratio, 0.75);
    }

    #[test]
    fn recompute_savings_no_compression() {
        let mut m = ChewingMetrics::default();
        m.input_tokens_raw = 50;
        m.input_tokens_compressed = 50;
        m.recompute_savings();
        assert_eq!(m.compression_ratio, 1.0);
        assert_eq!(m.savings_tokens, 0);
        assert_eq!(m.savings_ratio, 0.0);
    }

    #[test]
    fn recompute_savings_zero_division_safe() {
        let mut m = ChewingMetrics::default();
        m.input_tokens_raw = 0;
        m.input_tokens_compressed = 0;
        m.recompute_savings();
        // Pas de panic, pas de NaN.
        assert_eq!(m.compression_ratio, 1.0);
        assert_eq!(m.savings_tokens, 0);
        assert_eq!(m.savings_ratio, 0.0);
    }

    #[test]
    fn recompute_savings_pathological_inflation() {
        // Cas où le compressor "augmente" le texte (très improbable mais à gérer).
        let mut m = ChewingMetrics::default();
        m.input_tokens_raw = 50;
        m.input_tokens_compressed = 75;
        m.recompute_savings();
        assert!(m.compression_ratio > 1.0);
        assert_eq!(m.savings_tokens, 0); // saturating_sub évite l'underflow
        assert!(m.savings_ratio < 0.0); // signal clair que ça a empiré
    }

    #[test]
    fn recompute_total_slm_latency_sum() {
        let mut m = ChewingMetrics::default();
        m.analyzer_latency_ms = 100;
        m.compressor_latency_ms = 200;
        m.verifier_latency_ms = 300;
        m.router_latency_ms = 5; // exclu de la somme SLM
        m.recompute_total_slm_latency();
        assert_eq!(m.total_slm_latency_ms, 600);
    }

    #[test]
    fn default_is_zero() {
        let m = ChewingMetrics::default();
        assert_eq!(m.input_tokens_raw, 0);
        assert_eq!(m.compression_ratio, 0.0);
        assert_eq!(m.selected_llm, "");
        assert_eq!(m.total_pipeline_latency_ms, 0);
    }

    #[test]
    fn serde_roundtrip_stable() {
        let m = ChewingMetrics {
            input_tokens_raw: 100,
            input_tokens_compressed: 25,
            llm_tokens_generated: 30,
            compression_ratio: 0.25,
            savings_tokens: 75,
            savings_ratio: 0.75,
            analyzer_latency_ms: 500,
            router_latency_ms: 2,
            compressor_latency_ms: 800,
            llm_latency_ms: 1500,
            verifier_latency_ms: 400,
            total_slm_latency_ms: 1700,
            total_pipeline_latency_ms: 3200,
            selected_llm: "claude_claude-sonnet-4-6".into(),
            router_reason: "highest quality_score for FactualQa".into(),
            router_estimated_cost_cents: 2,
            // R3.S1 : 2 cents × 1000 = 2_000 µ¢.
            router_estimated_cost_microcents: 2_000,
            // S7 R2 — champs Mission Speculative
            verifier_skipped: Some(true),
            spec_drafter_latency_ms: 400,
            spec_verifier_latency_ms: 0,
            spec_savings_cents: 5,
            // R3.S1 : 5 cents × 1000 = 5_000 µ¢.
            spec_savings_microcents: 5_000,
            // R3.S3 — draft rejeté comme bruit.
            spec_noise_rejected: Some(false),
            // S8 R2 — Graceful degradation
            failure_mode: FailureMode::None,
            degradations_count: 0,
            failed_components: vec![],
        };
        let s = serde_json::to_string(&m).unwrap();
        let back: ChewingMetrics = serde_json::from_str(&s).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn s8_default_failure_mode_is_none() {
        let m = ChewingMetrics::default();
        assert_eq!(m.failure_mode, FailureMode::None);
        assert_eq!(m.degradations_count, 0);
        assert!(m.failed_components.is_empty());
    }

    #[test]
    fn s8_backward_compat_old_json_without_failure_fields() {
        // JSON pré-S8 (sans failure_mode, degradations_count, failed_components).
        let old_json = r#"{
            "input_tokens_raw": 100,
            "input_tokens_compressed": 25,
            "llm_tokens_generated": 30,
            "compression_ratio": 0.25,
            "savings_tokens": 75,
            "savings_ratio": 0.75,
            "analyzer_latency_ms": 0,
            "router_latency_ms": 0,
            "compressor_latency_ms": 0,
            "llm_latency_ms": 0,
            "verifier_latency_ms": 0,
            "total_slm_latency_ms": 0,
            "total_pipeline_latency_ms": 0,
            "selected_llm": "",
            "router_reason": "",
            "router_estimated_cost_cents": 0
        }"#;
        let parsed: ChewingMetrics = serde_json::from_str(old_json)
            .expect("backward-compat S8 doit fonctionner");
        assert_eq!(parsed.failure_mode, FailureMode::None);
        assert_eq!(parsed.degradations_count, 0);
        assert!(parsed.failed_components.is_empty());
    }

    #[test]
    fn s7_default_spec_fields_are_none_or_zero() {
        let m = ChewingMetrics::default();
        assert_eq!(m.verifier_skipped, None);
        assert_eq!(m.spec_drafter_latency_ms, 0);
        assert_eq!(m.spec_verifier_latency_ms, 0);
        assert_eq!(m.spec_savings_cents, 0);
    }

    #[test]
    fn s7_backward_compat_old_json_without_spec_fields() {
        // JSON pré-S7 sans les champs verifier_skipped + spec_*.
        // #[serde(default)] doit permettre la désérialisation correcte.
        let old_json = r#"{
            "input_tokens_raw": 100,
            "input_tokens_compressed": 25,
            "llm_tokens_generated": 30,
            "compression_ratio": 0.25,
            "savings_tokens": 75,
            "savings_ratio": 0.75,
            "analyzer_latency_ms": 0,
            "router_latency_ms": 0,
            "compressor_latency_ms": 0,
            "llm_latency_ms": 0,
            "verifier_latency_ms": 0,
            "total_slm_latency_ms": 0,
            "total_pipeline_latency_ms": 0,
            "selected_llm": "",
            "router_reason": "",
            "router_estimated_cost_cents": 0
        }"#;
        let parsed: ChewingMetrics = serde_json::from_str(old_json)
            .expect("backward-compat S7 doit fonctionner");
        assert_eq!(parsed.verifier_skipped, None);
        assert_eq!(parsed.spec_drafter_latency_ms, 0);
        assert_eq!(parsed.input_tokens_raw, 100);
    }

    #[test]
    fn serde_json_field_names_snake_case() {
        let m = ChewingMetrics::default();
        let s = serde_json::to_string(&m).unwrap();
        // Vérification que serde utilise le snake_case par défaut (cohérent avec
        // le reste du codebase et l'API publique consommée par les tests).
        assert!(s.contains("\"input_tokens_raw\":0"));
        assert!(s.contains("\"compression_ratio\":0"));
        assert!(s.contains("\"selected_llm\":\"\""));
    }
}
