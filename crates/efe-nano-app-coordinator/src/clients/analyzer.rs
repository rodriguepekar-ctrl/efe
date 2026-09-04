// SPDX-License-Identifier: UNLICENSED
//! Client du robot **Analyseur** (SmolLM2-360M Q5).
//!
//! Port par défaut : `8090`.

use std::path::Path;

use async_trait::async_trait;

use super::llama_server::{LlamaServerClient, RobotAssets, RobotHealth};
use super::{NanoRobotClient, RunMode};
use crate::context::CallContext;
use crate::error::{RobotError, RobotResult};
use crate::types::analyzer::{AnalyzerInput, AnalyzerOutput};

/// Client pour le robot Analyseur.
pub struct AnalyzerClient {
    inner: LlamaServerClient,
}

impl AnalyzerClient {
    /// Construit avec les assets canoniques (grammar + prompt depuis
    /// `assets/grammars/analyzer_v1.gbnf` et `assets/prompts/analyzer.txt`).
    pub fn from_assets(endpoint: impl Into<String>) -> RobotResult<Self> {
        let assets_root = assets_dir();
        let grammar = assets_root.join("grammars/analyzer_v1.gbnf");
        let prompt = assets_root.join("prompts/analyzer.txt");
        Ok(Self {
            inner: LlamaServerClient::from_assets(
                RobotAssets {
                    id: crate::robot_ids::ANALYZER,
                    schema_version: "1.0.0",
                    grammar_path: &grammar,
                    prompt_path: &prompt,
                },
                endpoint,
                /* max_tokens */ analyzer_max_output_tokens(),
                /* temperature */ 0.1,
            )?,
        })
    }

    /// Construit explicitement (test / customisation).
    #[must_use]
    pub fn from_helper(inner: LlamaServerClient) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl NanoRobotClient for AnalyzerClient {
    type Input = AnalyzerInput;
    type Output = AnalyzerOutput;

    fn id(&self) -> &'static str {
        crate::robot_ids::ANALYZER
    }
    fn schema_version(&self) -> &'static str {
        "1.0.0"
    }
    fn run_mode(&self) -> RunMode {
        RunMode::B
    }

    async fn invoke(
        &self,
        input: Self::Input,
        ctx: &CallContext,
    ) -> RobotResult<Self::Output> {
        use validator::Validate as _;
        // R6 — borne anti-échec AVANT la validation : le SmolLM2-360M échoue /
        // timeoute au-delà de ~1000 tokens (observé en live ; le validateur
        // `text` autorise pourtant jusqu'à 8000 car). L'analyzer ne fait que de
        // l'extraction d'entités — enrichissement HORS chemin critique : elles
        // n'alimentent NI le router NI le `must_preserve` du compresseur, juste
        // des phéromones de blackboard + des events UI. Un excerpt de tête suffit,
        // et borner avant validation rend l'analyzer robuste à N'IMPORTE quelle
        // taille d'entrée (au lieu d'échouer la validation au-delà de 8000 car).
        let input = bound_analyzer_input(input);
        // Validation input pré-vol (texte désormais sous la limite ; les autres
        // champs — `max_entities`, `source_agent` — restent validés).
        input.validate().map_err(|e| RobotError::SemanticViolation {
            robot: self.id(),
            issues: format!("input invalid: {e}"),
        })?;

        // Appel + parse JSON garanti par grammar.
        let output: AnalyzerOutput = self.inner.complete(&input, &input.text, ctx).await?;

        // Validation post-parse (les ranges que GBNF ne couvre pas).
        output.validate().map_err(|e| RobotError::SemanticViolation {
            robot: self.id(),
            issues: format!("post-parse invalid: {e}"),
        })?;
        output
            .validate_semantics()
            .map_err(|issues| RobotError::SemanticViolation {
                robot: self.id(),
                issues,
            })?;

        Ok(output)
    }

    async fn health(&self) -> RobotResult<RobotHealth> {
        self.inner.health().await
    }
}

/// Localise `assets/` relativement au manifest du crate.
pub(crate) fn assets_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets")
}

// ─────────────────────────────────────────────────────────────────────────
// R6 — borne d'entrée anti-échec (SmolLM2-360M)
// ─────────────────────────────────────────────────────────────────────────

/// Limite d'entrée fiable du SmolLM2-360M, en **tokens estimés**. Au-delà,
/// l'appel unique échoue/timeoute (mesuré en live : OK ~720 t, KO ~1030 t). On
/// reste nettement en dessous. Override : `EFE_ANALYZER_MAX_TOKENS` (plancher 32).
const SAFE_ANALYZER_TOKENS: usize = 512;

/// Budget de caractères de l'entrée analyzer (≈ 4 car/token, cohérent avec
/// `estimate_tokens`). Lu de `EFE_ANALYZER_MAX_TOKENS` (tokens) avec défaut
/// [`SAFE_ANALYZER_TOKENS`].
fn safe_analyzer_chars() -> usize {
    let toks = std::env::var("EFE_ANALYZER_MAX_TOKENS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n >= 32)
        .unwrap_or(SAFE_ANALYZER_TOKENS);
    toks.saturating_mul(4)
}

/// **C-B** — plafond de tokens de **SORTIE** (`n_predict`) de l'analyzer. **À ne
/// PAS confondre** avec `EFE_ANALYZER_MAX_TOKENS`, qui borne l'**ENTRÉE**. Le vrai
/// cap du nombre d'entités (donc de la longueur de sortie) est la **grammaire**
/// (`analyzer_v1.gbnf` : `entity){0,4}`) : le décodage s'arrête à la complétion du
/// JSON. Ce plafond n'est qu'un **backstop d'hygiène** (évite un décodage emballé
/// si le modèle refuse l'EOS) + un knob de tuning. **1024 → 512** : largement
/// au-dessus d'une sortie complète à 5 entités (~215-300 t), bien sous l'ancien
/// plafond. Override : `EFE_ANALYZER_MAX_OUTPUT_TOKENS` (plancher 64).
const ANALYZER_MAX_OUTPUT_TOKENS: u32 = 512;

fn analyzer_max_output_tokens() -> u32 {
    std::env::var("EFE_ANALYZER_MAX_OUTPUT_TOKENS")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|&n| n >= 64)
        .unwrap_or(ANALYZER_MAX_OUTPUT_TOKENS)
}

/// Borne le `text` de l'entrée analyzer à [`safe_analyzer_chars`] (troncature de
/// TÊTE sur frontière de codepoint — jamais au milieu d'un caractère). No-op si
/// déjà sous la limite. L'analyzer étant observationnel (entités), un excerpt de
/// tête est un échantillon représentatif suffisant.
fn bound_analyzer_input(mut input: AnalyzerInput) -> AnalyzerInput {
    let max = safe_analyzer_chars();
    if input.text.chars().count() > max {
        input.text = input.text.chars().take(max).collect();
    }
    input
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_dir_exists() {
        let d = assets_dir();
        assert!(
            d.join("grammars").exists() || !d.exists(),
            "assets/grammars/ should exist when assets/ exists"
        );
    }

    // ── R6 — borne d'entrée anti-échec ─────────────────────────────────────

    fn ai(text: &str) -> AnalyzerInput {
        AnalyzerInput {
            text: text.to_string(),
            max_entities: 20,
            source_agent: None,
        }
    }

    #[test]
    fn safe_analyzer_chars_default_is_below_failure_point() {
        std::env::remove_var("EFE_ANALYZER_MAX_TOKENS");
        // Défaut = 512 t × 4 = 2048 car, bien sous le ~1030 t (~4120 car) d'échec.
        assert_eq!(safe_analyzer_chars(), SAFE_ANALYZER_TOKENS * 4);
        assert!(safe_analyzer_chars() < 4000, "marge sous le point d'échec live");
    }

    #[test]
    fn bound_truncates_long_text_on_codepoint_boundary() {
        std::env::remove_var("EFE_ANALYZER_MAX_TOKENS");
        // « é » = 1 codepoint / 2 octets → vérifie qu'on ne coupe pas un caractère.
        let bounded = bound_analyzer_input(ai(&"é".repeat(10_000)));
        assert_eq!(
            bounded.text.chars().count(),
            SAFE_ANALYZER_TOKENS * 4,
            "tronqué au budget exact (en CARACTÈRES)"
        );
        assert!(bounded.text.chars().all(|c| c == 'é'), "aucun codepoint coupé");
    }

    #[test]
    fn bound_leaves_short_text_untouched() {
        let bounded = bound_analyzer_input(ai("Quelle est la capitale ?"));
        assert_eq!(bounded.text, "Quelle est la capitale ?");
    }

    /// Un input > 8000 car (qui échouait la validation `max=8000` → analyzer KO)
    /// est borné AVANT → passe la validation et ne casse plus.
    #[test]
    fn oversized_input_is_bounded_then_validates() {
        use validator::Validate as _;
        std::env::remove_var("EFE_ANALYZER_MAX_TOKENS");
        let bounded = bound_analyzer_input(ai(&"x".repeat(10_000)));
        assert!(
            bounded.text.chars().count() <= 8000,
            "sous le max du validateur"
        );
        assert!(bounded.validate().is_ok(), "borné → input valide");
    }

    #[test]
    fn id_and_version_are_stable() {
        // On instancie inner manuellement pour ne pas dépendre des fichiers.
        let inner = LlamaServerClient {
            id: crate::robot_ids::ANALYZER,
            schema_version: "1.0.0",
            endpoint: "http://127.0.0.1:8090".into(),
            grammar: String::new(),
            system_prompt: String::new(),
            max_tokens: 512,
            temperature: 0.1,
            timeout: std::time::Duration::from_secs(1),
            http: reqwest::Client::new(),
        };
        let client = AnalyzerClient::from_helper(inner);
        assert_eq!(client.id(), "analyzer");
        assert_eq!(client.schema_version(), "1.0.0");
        assert_eq!(client.run_mode(), RunMode::B);
    }

    // ── C-B (right-size analyzer) ──────────────────────────────────────────

    #[test]
    fn cb_analyzer_max_output_tokens_default_is_512() {
        // Plafond de SORTIE 1024 → 512 (backstop ; le vrai cap = grammaire). On
        // asserte la CONSTANTE (immune à l'env) puis le défaut de la fonction.
        assert_eq!(ANALYZER_MAX_OUTPUT_TOKENS, 512);
        std::env::remove_var("EFE_ANALYZER_MAX_OUTPUT_TOKENS");
        assert_eq!(analyzer_max_output_tokens(), 512);
    }

    /// La grammaire d'asset borne les entités à 5 (`{0,4}`), pas 20 — garde-fou de
    /// couplage asset↔intention (anti-régression si on régénère depuis le schéma).
    #[test]
    fn cb_grammar_asset_caps_entities_at_five() {
        let gbnf = std::fs::read_to_string(assets_dir().join("grammars/analyzer_v1.gbnf"))
            .expect("grammar analyzer lisible");
        assert!(
            gbnf.contains("entity){0,4}"),
            "analyzer_v1.gbnf doit borner entities à {{0,4}} (C-B right-size)"
        );
        assert!(
            !gbnf.contains("entity){0,19}"),
            "l'ancien cap {{0,19}} (20 entités) ne doit plus exister"
        );
    }
}
