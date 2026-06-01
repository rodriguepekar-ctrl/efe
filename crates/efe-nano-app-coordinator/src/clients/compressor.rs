// SPDX-License-Identifier: UNLICENSED
//! Client du robot **Compresseur** (Llama-3.2-1B Q5).
//!
//! Port par défaut : `8092`.

use async_trait::async_trait;

use super::llama_server::{LlamaServerClient, RobotHealth};
use super::{NanoRobotClient, RunMode};
use crate::clients::analyzer::assets_dir;
use crate::context::CallContext;
use crate::error::{RobotError, RobotResult};
use crate::types::compressor::{CompressorInput, CompressorOutput};

/// Client pour le robot Compresseur.
pub struct CompressorClient {
    inner: LlamaServerClient,
}

impl CompressorClient {
    /// Construit avec les assets canoniques.
    pub fn from_assets(endpoint: impl Into<String>) -> RobotResult<Self> {
        let assets = assets_dir();
        Ok(Self {
            inner: LlamaServerClient::from_assets(
                crate::robot_ids::COMPRESSOR,
                "1.0.0",
                endpoint,
                assets.join("grammars/compressor_v1.gbnf"),
                assets.join("prompts/compressor.txt"),
                /* max_tokens */ 1536,
                /* temperature */ 0.2,
            )?,
        })
    }

    /// Construit explicitement.
    #[must_use]
    pub fn from_helper(inner: LlamaServerClient) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl NanoRobotClient for CompressorClient {
    type Input = CompressorInput;
    type Output = CompressorOutput;

    fn id(&self) -> &'static str {
        crate::robot_ids::COMPRESSOR
    }
    fn schema_version(&self) -> &'static str {
        "1.0.0"
    }
    fn run_mode(&self) -> RunMode {
        RunMode::C
    }

    async fn invoke(
        &self,
        input: Self::Input,
        ctx: &CallContext,
    ) -> RobotResult<Self::Output> {
        use validator::Validate as _;
        input.validate().map_err(|e| RobotError::SemanticViolation {
            robot: self.id(),
            issues: format!("input invalid: {e}"),
        })?;

        // ── Levier 3 — dispatch long-contexte ───────────────────────────────
        // Un appel unique dégénère (recrache son placeholder) ou timeoute (5s)
        // au-delà de ~700 tokens sur le Llama-3.2-1B. Si l'entrée dépasse
        // `SAFE_CHUNK_TOKENS`, on bascule en compression **map-reduce** (chunked
        // + réduction hiérarchique, cf. `compress_long`). Sinon, appel unique
        // (chemin historique préservé bit-à-bit).
        let output = if estimate_tokens(&input.text) > safe_chunk_tokens() {
            self.compress_long(&input, ctx).await?
        } else {
            self.compress_once(&input, ctx).await?
        };

        output.validate().map_err(|e| RobotError::SemanticViolation {
            robot: self.id(),
            issues: format!("post-parse invalid: {e}"),
        })?;

        // Vérification stricte : must_preserve respecté (sur le résultat FINAL ;
        // un chunk intermédiaire ne contient pas forcément toutes les entités).
        output
            .validate_preservation(&input.must_preserve)
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

// ─────────────────────────────────────────────────────────────────────────
// Levier 3 — compression long-contexte (map-reduce hiérarchique)
// ─────────────────────────────────────────────────────────────────────────

impl CompressorClient {
    /// **Un appel** de compression : valide l'input, sérialise, `complete`,
    /// valide le **schéma** de sortie. **Ne valide PAS la préservation** — c'est
    /// réservé au résultat FINAL (un chunk intermédiaire ne contient pas
    /// forcément toutes les entités `must_preserve`).
    async fn compress_once(
        &self,
        input: &CompressorInput,
        ctx: &CallContext,
    ) -> RobotResult<CompressorOutput> {
        use validator::Validate as _;
        input.validate().map_err(|e| RobotError::SemanticViolation {
            robot: crate::robot_ids::COMPRESSOR,
            issues: format!("chunk input invalid: {e}"),
        })?;
        let user_text = serde_json::to_string(input).map_err(|e| RobotError::Config {
            robot: crate::robot_ids::COMPRESSOR,
            reason: format!("input serialization failed: {e}"),
        })?;
        // R5 corr.3 — grammar DYNAMIQUE : resserre le cap du `summary` à
        // `summary_char_budget(target_words)` pour forcer la brièveté que le 1B
        // n'honore pas via le prompt. S'applique à CHAQUE appel (chunks du MAP
        // ET resserrage final), chacun avec son `target_words` propre.
        let grammar = dynamic_compressor_grammar(&self.inner.grammar, input.target_words);
        let output: CompressorOutput = self
            .inner
            .complete_with_grammar(input, &user_text, &grammar, ctx)
            .await?;
        output.validate().map_err(|e| RobotError::SemanticViolation {
            robot: crate::robot_ids::COMPRESSOR,
            issues: format!("chunk post-parse invalid: {e}"),
        })?;
        Ok(output)
    }

    /// **Map-reduce** pour input long (> [`SAFE_CHUNK_TOKENS`]).
    ///
    /// 1. **MAP** : découpe en morceaux que le 1B traite de façon fiable, et
    ///    compresse chacun (séquentiel — llama-server `--parallel 1`).
    /// 2. **REDUCE** : concatène ; si la concaténation dépasse encore le seuil,
    ///    on récurse (itératif, borné par [`MAX_REDUCE_PASSES`]).
    /// 3. **Resserrage final** → un `CompressorOutput` unique.
    ///
    /// Robustesse : un morceau dégénéré (placeholder) ou échoué (timeout/
    /// transport) dégrade vers un **extrait tronqué du brut** — on préserve
    /// l'information sans propager de garbage. Un resserrage final qui échoue
    /// produit une **sortie synthétique** à partir du texte réduit.
    async fn compress_long(
        &self,
        input: &CompressorInput,
        ctx: &CallContext,
    ) -> RobotResult<CompressorOutput> {
        let source_words = input.text.split_whitespace().count().max(1);
        let mut current = input.text.clone();
        let mut passes = 0usize;

        let max_passes = max_reduce_passes();
        let chunk_tokens = safe_chunk_tokens();
        let overlap = chunk_overlap_tokens();

        // ── REDUCE itératif (chaque passe fait un MAP) ──
        while passes < max_passes {
            let chunks = split_into_chunks_overlap(&current, chunk_tokens, overlap);
            if chunks.len() <= 1 {
                break; // tient sous le seuil → resserrage final
            }
            let per_chunk_target =
                ((input.target_words as usize) / chunks.len()).clamp(10, 500) as u32;

            let mut partials: Vec<String> = Vec::with_capacity(chunks.len());
            for chunk in &chunks {
                let ci = CompressorInput {
                    text: chunk.clone(),
                    target_words: per_chunk_target,
                    must_preserve: Vec::new(), // pas d'enforcement par-chunk
                    style: input.style,
                };
                let partial = match self.compress_once(&ci, ctx).await {
                    Ok(out) if !is_degenerate_summary(&out.summary) => out.summary,
                    // dégénéré OU échec → extrait tronqué du brut.
                    _ => truncate_chars(chunk, 240),
                };
                partials.push(partial);
            }
            current = partials.join(" ");
            passes += 1;
        }

        // ── Resserrage final ──
        let final_input = CompressorInput {
            text: ensure_min_len(&current),
            target_words: input.target_words,
            must_preserve: input.must_preserve.clone(),
            style: input.style,
        };
        let out = match self.compress_once(&final_input, ctx).await {
            Ok(o) if !is_degenerate_summary(&o.summary) => o,
            _ => synthesize_output(&current, source_words),
        };
        Ok(out)
    }
}

// ─────────────────────────────────────────────────────────────────────────
// R5 corr.3 — Brièveté forcée par la grammaire (cap du summary ∝ target_words)
// ─────────────────────────────────────────────────────────────────────────

/// Caractères budgétés par mot-cible pour le `summary` (~1.3× le nb de mots :
/// généreux, pour ne PAS tronquer un résumé bien formé).
const SUMMARY_CHARS_PER_WORD: usize = 8;
/// Plancher du cap (≈ 30 mots — la cible minimale de `proportional_target_words`).
const SUMMARY_CHARS_MIN: usize = 120;
/// Plafond du cap = cap historique de `compressor_v1.gbnf` (jamais pire qu'avant).
const SUMMARY_CHARS_MAX: usize = 500;

/// Budget de caractères du `summary`, dérivé de `target_words`. Le 1B ne respecte
/// pas `target_words` (il remplit vers le cap de la grammaire) ; on RÉTRÉCIT donc
/// le cap proportionnellement à la cible → brièveté forcée token par token.
/// Borné `[120, 500]` : jamais sous ~30 mots, jamais au-dessus du cap historique
/// (pour les grandes cibles ≥ ~62 mots, sature à 500 = comportement actuel).
fn summary_char_budget(target_words: u32) -> usize {
    ((target_words as usize) * SUMMARY_CHARS_PER_WORD).clamp(SUMMARY_CHARS_MIN, SUMMARY_CHARS_MAX)
}

/// Resserre le cap du `summary` dans la grammaire : substitue le littéral exact
/// `char{1,500}` (règle `long-string` de `compressor_v1.gbnf`, UNIQUE — les
/// `short`/`tiny-string` utilisent `{1,100}`/`{1,50}`) par `char{1,N}` avec
/// `N = summary_char_budget(target_words)`. Motif absent (grammaire altérée) →
/// grammaire inchangée (dégradation sûre = comportement actuel).
fn dynamic_compressor_grammar(base: &str, target_words: u32) -> String {
    let n = summary_char_budget(target_words);
    base.replace("char{1,500}", &format!("char{{1,{n}}}"))
}

/// Au-dessus de ce seuil (tokens estimés), `invoke` bascule en map-reduce
/// (N appels SLM **séquentiels** via `compress_long`). En-dessous : **un seul**
/// `compress_once`.
///
/// **L1 (levier latence, suite du spike H0.2) — relevé 300 → 512.** Le 300
/// était une marge **CPU** : à l'époque, un appel sur ~300+ tokens timeoutait
/// (per-call 5 s, `llama_server.rs`) sur le Llama-3.2-1B en CPU, donc on
/// chunkait tôt pour rester fiable. Mesure H0.2 : sur GPU (Metal) un appel
/// finit en ~1,5-2 s à 300 t et ~2-3 s à 512 t — très loin du timeout 5 s ; et
/// le ctx-size compresseur (4096, `scripts/slm/lib_common.sh`) contient
/// largement 512 t d'entrée + ~500 c de sortie + prompt système. Conséquence :
/// toute la bande **300-512 t** (la majorité des prompts « redundant » une fois
/// la question ré-injectée par R7) passe de **3-4 appels série** à **un seul**
/// → gain latence direct, sans toucher l'infra (`--parallel`) ni le matériel.
///
/// 512 reste **sous** le plafond fiable mesuré (~700 t post-fix-prompt). Une
/// dérive éventuelle (prompt borderline qui dégénère) dégrade proprement via le
/// timeout + le fallback `truncate_chars` de `compress_long` — jamais de
/// garbage propagé. Override : env `EFE_COMPRESSOR_CHUNK_TOKENS` (plancher 12) ;
/// repasser à `300` restaure exactement le comportement pré-L1 (ex. parc CPU).
const SAFE_CHUNK_TOKENS: usize = 512;
/// Garde-fou : nombre max de passes de réduction hiérarchique (anti-boucle).
/// Override : env `EFE_COMPRESSOR_MAX_REDUCE_PASSES`.
const MAX_REDUCE_PASSES: usize = 4;
/// Recouvrement entre chunks consécutifs (~30 tokens) : un fait à cheval sur
/// une frontière apparaît dans les deux chunks et n'est pas perdu. C'est un
/// plancher **absolu** (et non plus « ~10 % de `SAFE_CHUNK_TOKENS` » : depuis L1
/// le seuil vaut 512, et un overlap fixe ~30 t suffit à couvrir une frontière
/// tout en gardant le MAP léger — moins de tokens dupliqués re-compressés).
/// N'affecte QUE les prompts > 512 t (les seuls qui chunkent encore). Override :
/// env `EFE_COMPRESSOR_CHUNK_OVERLAP_TOKENS` (0 = aucun recouvrement).
const CHUNK_OVERLAP_TOKENS: usize = 30;
/// `CompressorInput.text` exige ≥ 50 chars : on ne produit pas de chunk plus court.
const MIN_CHUNK_CHARS: usize = 50;

/// Lit un `usize` depuis une variable d'env, avec défaut + plancher.
fn env_usize(var: &str, default: usize, min: usize) -> usize {
    std::env::var(var)
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n >= min)
        .unwrap_or(default)
}

/// Seuil de bascule map-reduce (tokens), configurable via env.
fn safe_chunk_tokens() -> usize {
    env_usize("EFE_COMPRESSOR_CHUNK_TOKENS", SAFE_CHUNK_TOKENS, 12)
}
/// Nombre max de passes de réduction, configurable via env.
fn max_reduce_passes() -> usize {
    env_usize("EFE_COMPRESSOR_MAX_REDUCE_PASSES", MAX_REDUCE_PASSES, 1)
}
/// Recouvrement inter-chunks (tokens), configurable via env (0 = pas d'overlap).
fn chunk_overlap_tokens() -> usize {
    env_usize("EFE_COMPRESSOR_CHUNK_OVERLAP_TOKENS", CHUNK_OVERLAP_TOKENS, 0)
}

/// Estime le nombre de tokens (~chars/4, aligné sur `ChewingMetrics::estimate_tokens`).
fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// Tronque à `max` caractères (sûr Unicode).
fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Garantit ≥ [`MIN_CHUNK_CHARS`] (contrainte `CompressorInput.text`). Pad avec
/// des espaces si trop court (cas dégénéré rare).
fn ensure_min_len(s: &str) -> String {
    let n = s.chars().count();
    if n >= MIN_CHUNK_CHARS {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(MIN_CHUNK_CHARS - n))
    }
}

/// Détecte une sortie compresseur **dégénérée** (placeholder de template
/// recraché, ou vide). Miroir local du garde moteur
/// (`efe_coordinator::workflows::final_prompt_from_compression`).
fn is_degenerate_summary(summary: &str) -> bool {
    let s = summary.trim();
    s.is_empty() || s.contains("<the compressed text>") || s.contains("<source text>")
}

/// Découpe `text` en morceaux de ~`max_tokens` sur frontières de **mots** (jamais
/// au milieu d'un mot), avec un **recouvrement** de ~`overlap_tokens` entre
/// morceaux consécutifs — un fait à cheval sur une frontière apparaît alors dans
/// les deux chunks et n'est pas perdu. Le dernier morceau trop petit
/// (< [`MIN_CHUNK_CHARS`]) est fusionné avec le précédent. La progression est
/// garantie (≥ 1 mot consommé par chunk → pas de boucle infinie). **Pur**.
pub(crate) fn split_into_chunks_overlap(
    text: &str,
    max_tokens: usize,
    overlap_tokens: usize,
) -> Vec<String> {
    let max_chars = (max_tokens * 4).max(MIN_CHUNK_CHARS);
    let overlap_chars = overlap_tokens.saturating_mul(4);
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return vec![trimmed.to_string()];
    }

    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut start = 0usize;

    while start < words.len() {
        // Empile les mots depuis `start` jusqu'à ~max_chars.
        let mut current = String::new();
        let mut end = start;
        while end < words.len() {
            let projected = current.chars().count() + 1 + words[end].chars().count();
            if !current.is_empty() && projected > max_chars {
                break;
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(words[end]);
            end += 1;
        }
        chunks.push(current);
        if end >= words.len() {
            break;
        }
        // Prochain départ : recule de ~overlap_chars (en mots) depuis `end`,
        // tout en garantissant la progression (au moins 1 mot consommé).
        let mut back = end;
        let mut acc = 0usize;
        while back > start + 1 && acc < overlap_chars {
            back -= 1;
            acc += words[back].chars().count() + 1;
        }
        start = back.max(start + 1);
    }

    // Fusionne un dernier morceau trop petit (contrainte ≥ 50 chars).
    if chunks.len() >= 2 {
        let last_short = chunks
            .last()
            .map(|c| c.chars().count() < MIN_CHUNK_CHARS)
            .unwrap_or(false);
        if last_short {
            let last = chunks.pop().expect("len >= 2");
            let prev = chunks.last_mut().expect("len >= 1");
            prev.push(' ');
            prev.push_str(&last);
        }
    }
    chunks
}

/// Sortie synthétique quand le resserrage final échoue : le texte réduit tronqué
/// (cap 2000 chars = borne `CompressorOutput.summary`) devient le résumé.
fn synthesize_output(reduced: &str, source_words: usize) -> CompressorOutput {
    let summary = truncate_chars(reduced.trim(), 2000);
    let summary_words = summary.split_whitespace().count().clamp(1, 500);
    let ratio = ((source_words as f32) / (summary_words as f32)).clamp(1.0, 50.0);
    CompressorOutput {
        summary,
        word_count: summary_words as u32,
        preserved: Vec::new(),
        omitted_for_brevity: Vec::new(),
        compression_ratio: ratio,
    }
}

#[cfg(test)]
mod chunking_tests {
    use super::*;

    // ── R5 corr.3 — brièveté forcée par grammaire dynamique ────────────────

    #[test]
    fn summary_char_budget_scales_and_clamps() {
        assert_eq!(summary_char_budget(30), 240, "30 mots × 8 = 240");
        assert_eq!(summary_char_budget(10), 120, "sous le plancher → 120");
        assert_eq!(summary_char_budget(0), 120, "0 → plancher 120");
        assert_eq!(summary_char_budget(62), 496, "62 × 8 = 496");
        assert_eq!(summary_char_budget(100), 500, "au-dessus → plafond 500");
        assert_eq!(summary_char_budget(500), 500, "grande cible → 500 (inchangé)");
    }

    #[test]
    fn dynamic_grammar_tightens_only_long_string_cap() {
        // Mini-grammaire reproduisant les 3 règles de compressor_v1.gbnf.
        let base = "long-string ::= \"\\\"\" char{1,500} \"\\\"\"\n\
                    short-string ::= \"\\\"\" char{1,100} \"\\\"\"\n\
                    tiny-string ::= \"\\\"\" char{1,50} \"\\\"\"";
        // Petite cible → cap resserré, et SEUL long-string touché.
        let g = dynamic_compressor_grammar(base, 30);
        assert!(g.contains("char{1,240}"), "long-string resserré à 240");
        assert!(g.contains("char{1,100}"), "short-string {{1,100}} intact");
        assert!(g.contains("char{1,50}"), "tiny-string {{1,50}} intact");
        assert!(!g.contains("char{1,500}"), "l'ancien cap 500 a disparu");
        // Grande cible → 500 → substitution no-op (identique au brut).
        assert_eq!(dynamic_compressor_grammar(base, 500), base, "≥62 mots → inchangé");
        // Motif absent → grammaire inchangée (dégradation sûre).
        assert_eq!(dynamic_compressor_grammar("root ::= x", 30), "root ::= x");
    }

    /// La grammaire RÉELLE de l'asset contient bien le motif ciblé (sinon la
    /// substitution serait un no-op silencieux). Garde-fou de couplage asset↔code.
    #[test]
    fn real_asset_grammar_contains_target_pattern() {
        let path = assets_dir().join("grammars/compressor_v1.gbnf");
        let gbnf = std::fs::read_to_string(&path).expect("grammar asset lisible");
        assert!(
            gbnf.contains("char{1,500}"),
            "compressor_v1.gbnf doit contenir `char{{1,500}}` (cap long-string) — \
             sinon dynamic_compressor_grammar ne resserre rien"
        );
        // Et la substitution produit un cap plus petit pour une petite cible.
        assert!(dynamic_compressor_grammar(&gbnf, 30).contains("char{1,240}"));
    }

    #[test]
    fn estimate_tokens_matches_chars_over_4_ceil() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2); // 5/4 → ceil 2
        assert_eq!(estimate_tokens(&"x".repeat(1200)), 300);
    }

    // ── L1 (levier latence) — seuil de bascule map-reduce relevé 300 → 512 ──
    // Ces tests verrouillent la NOUVELLE frontière de dispatch (`invoke` :
    // `estimate_tokens(text) > safe_chunk_tokens()`). Ils comparent à la
    // CONSTANTE `SAFE_CHUNK_TOKENS` (et non à `safe_chunk_tokens()`, qui lit
    // l'env) → immunes à un `EFE_COMPRESSOR_CHUNK_TOKENS` présent dans
    // l'environnement de test et au parallélisme cargo. La mécanique d'override
    // est déjà couverte par `env_usize_parses_with_default_and_floor`.

    #[test]
    fn l1_safe_chunk_threshold_default_is_512() {
        // Le défaut DOIT être 512 (était 300, marge CPU). Régression-lock du
        // changement L1 lui-même.
        assert_eq!(SAFE_CHUNK_TOKENS, 512, "L1 : seuil map-reduce = 512 tokens");
    }

    #[test]
    fn l1_midband_prompt_now_takes_single_call_path() {
        // Prompt ~1700 chars (cf. `varied_backend` du harness éco H0.1) :
        // estimate_tokens = ceil(1700/4) = 425 tokens.
        //   AVANT L1 (seuil 300) : 425 > 300 → map-reduce (≈3 appels série).
        //   APRÈS L1 (seuil 512) : 425 ≤ 512 → un seul `compress_once`.
        let prompt = "x".repeat(1700);
        let toks = estimate_tokens(&prompt);
        assert_eq!(toks, 425, "modèle d'estimation chars/4");
        assert!(toks > 300, "aurait map-reducé sous l'ancien seuil 300");
        assert!(
            toks <= SAFE_CHUNK_TOKENS,
            "L1 : {toks} t ≤ {SAFE_CHUNK_TOKENS} → appel unique (plus de map-reduce)"
        );
    }

    #[test]
    fn l1_does_not_disable_chunking_for_genuinely_long_prompts() {
        // L1 RELÈVE le seuil ; il ne SUPPRIME pas le map-reduce. Un prompt
        // franchement long (> 512 t) doit encore franchir le seuil de bascule
        // ET se découper en plusieurs chunks.
        let prompt = "mot ".repeat(700); // 2800 chars, frontières de mots réelles
        let toks = estimate_tokens(&prompt);
        assert_eq!(toks, 700, "2800 chars / 4 = 700 tokens");
        assert!(
            toks > SAFE_CHUNK_TOKENS,
            "{toks} t > {SAFE_CHUNK_TOKENS} → dispatch map-reduce conservé"
        );
        let chunks = split_into_chunks_overlap(&prompt, SAFE_CHUNK_TOKENS, 0);
        assert!(
            chunks.len() >= 2,
            "un prompt > seuil se découpe en plusieurs chunks, got {}",
            chunks.len()
        );
    }

    #[test]
    fn l1_threshold_boundary_is_inclusive_single_call() {
        // Frontière exacte : `invoke` map-reduce SSI `tokens > seuil` (strict).
        // Donc un prompt pile à 512 t reste en appel unique ; 513 t bascule.
        let at = "z".repeat(SAFE_CHUNK_TOKENS * 4); // 2048 chars → 512 t pile
        assert_eq!(estimate_tokens(&at), SAFE_CHUNK_TOKENS);
        assert!(
            !(estimate_tokens(&at) > SAFE_CHUNK_TOKENS),
            "pile au seuil → appel unique (comparaison stricte `>`)"
        );
        let over = "z".repeat(SAFE_CHUNK_TOKENS * 4 + 4); // +1 token
        assert!(
            estimate_tokens(&over) > SAFE_CHUNK_TOKENS,
            "un token au-dessus du seuil → bascule map-reduce"
        );
    }

    #[test]
    fn short_text_is_a_single_chunk() {
        let t = "Phrase courte qui tient largement sous le seuil de chunking.";
        let chunks = split_into_chunks_overlap(t, 300, 0);
        assert_eq!(chunks, vec![t.to_string()]);
    }

    #[test]
    fn long_text_splits_into_multiple_bounded_chunks() {
        let t = "mot ".repeat(1000); // ~4000 chars
        let chunks = split_into_chunks_overlap(&t, 300, 0); // max_chars = 1200
        assert!(chunks.len() > 1, "doit produire plusieurs chunks");
        for c in &chunks {
            assert!(
                c.chars().count() <= 1200,
                "chunk ≤ 1200 chars, got {}",
                c.chars().count()
            );
            assert!(c.chars().count() >= MIN_CHUNK_CHARS, "chunk ≥ 50 chars");
        }
    }

    #[test]
    fn split_never_breaks_a_word_and_preserves_word_order() {
        let t = "alpha bravo charlie delta echo foxtrot golf hotel india juliet ".repeat(50);
        let chunks = split_into_chunks_overlap(&t, 50, 0); // petits chunks → beaucoup
        let src: Vec<&str> = t.split_whitespace().collect();
        let rebuilt: Vec<String> = chunks
            .iter()
            .flat_map(|c| c.split_whitespace().map(String::from))
            .collect();
        assert_eq!(rebuilt.len(), src.len(), "aucun mot perdu/dupliqué");
        for (a, b) in src.iter().zip(rebuilt.iter()) {
            assert_eq!(*a, b, "ordre des mots préservé");
        }
    }

    #[test]
    fn tiny_trailing_chunk_is_merged() {
        let big = "x".repeat(1190);
        let small = "yy";
        let t = format!("{big} {small}");
        let chunks = split_into_chunks_overlap(&t, 300, 0); // max_chars 1200
        assert!(
            chunks.iter().all(|c| c.chars().count() >= MIN_CHUNK_CHARS),
            "aucun chunk < 50 chars : {:?}",
            chunks.iter().map(|c| c.chars().count()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn is_degenerate_catches_placeholders_and_empty() {
        assert!(is_degenerate_summary("<the compressed text>"));
        assert!(is_degenerate_summary("résumé <source text> fin"));
        assert!(is_degenerate_summary(""));
        assert!(is_degenerate_summary("   \n\t "));
        assert!(!is_degenerate_summary("Paris est la capitale."));
    }

    #[test]
    fn ensure_min_len_pads_short_inputs() {
        assert_eq!(ensure_min_len(&"x".repeat(60)).chars().count(), 60);
        assert_eq!(ensure_min_len("abc").chars().count(), MIN_CHUNK_CHARS);
    }

    #[test]
    fn synthesize_output_is_schema_valid() {
        use validator::Validate as _;
        let reduced = "Paris est la capitale de la France ".repeat(100);
        let out = synthesize_output(&reduced, 1000);
        assert!(out.validate().is_ok(), "sortie synthétique doit valider le schéma");
        assert!(out.summary.chars().count() <= 2000);
        assert!(out.word_count >= 1 && out.word_count <= 500);
        assert!(out.compression_ratio >= 1.0 && out.compression_ratio <= 50.0);
    }

    #[test]
    fn overlap_zero_is_an_exact_partition() {
        let t = "un deux trois quatre cinq six sept huit neuf dix ".repeat(40);
        let chunks = split_into_chunks_overlap(&t, 50, 0);
        let src: Vec<&str> = t.split_whitespace().collect();
        let rebuilt: Vec<String> = chunks
            .iter()
            .flat_map(|c| c.split_whitespace().map(String::from))
            .collect();
        assert_eq!(rebuilt.len(), src.len(), "overlap=0 → aucune duplication");
    }

    #[test]
    fn overlap_repeats_boundary_words() {
        let t = "alpha bravo charlie delta echo foxtrot golf hotel india juliet ".repeat(40);
        let count = |v: &[String]| -> usize {
            v.iter().map(|c| c.split_whitespace().count()).sum()
        };
        let no_ov = split_into_chunks_overlap(&t, 50, 0);
        let with_ov = split_into_chunks_overlap(&t, 50, 20);
        assert!(no_ov.len() > 1 && with_ov.len() > 1, "plusieurs chunks attendus");
        assert!(
            count(&with_ov) > count(&no_ov),
            "l'overlap duplique des mots aux frontières : {} (overlap) vs {} (sans)",
            count(&with_ov),
            count(&no_ov)
        );
    }

    #[test]
    fn huge_overlap_still_terminates_no_infinite_loop() {
        // Overlap ≥ taille de chunk : la progression (≥ 1 mot/chunk) doit
        // garantir la terminaison.
        let t = "mot ".repeat(2000);
        let chunks = split_into_chunks_overlap(&t, 50, 10_000);
        assert!(!chunks.is_empty());
        assert!(chunks.len() < 5000, "doit terminer, got {} chunks", chunks.len());
    }

    #[test]
    fn env_usize_parses_with_default_and_floor() {
        let var = "EFE_TEST_ONLY_USIZE_XYZ"; // nom unique → aucun code prod ne le lit
        std::env::remove_var(var);
        assert_eq!(env_usize(var, 7, 1), 7, "absent → défaut");
        std::env::set_var(var, "42");
        assert_eq!(env_usize(var, 7, 1), 42, "override");
        std::env::set_var(var, "abc");
        assert_eq!(env_usize(var, 7, 1), 7, "invalide → défaut");
        std::env::set_var(var, "3");
        assert_eq!(env_usize(var, 7, 5), 7, "sous le plancher → défaut");
        std::env::remove_var(var);
    }
}
