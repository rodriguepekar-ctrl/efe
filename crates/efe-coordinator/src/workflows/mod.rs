// SPDX-License-Identifier: UNLICENSED
//! Workflows haut-niveau du Coordinator.
//!
//! - [`run_mission_a`] : pré-mâcher pour 1 LLM.
//! - [`run_mission_b`] : fan-out N LLMs + vote.
//! - [`run_speculative`] : drafter + verifier.

mod mission_a;
mod mission_b;
mod speculative;
pub(crate) mod slm_guard;

pub use mission_a::{run_mission_a, run_mission_a_with_swarm_id};
pub use mission_b::{run_mission_b, run_mission_b_with_swarm_id};
pub use speculative::{run_speculative, run_speculative_with_swarm_id};

/// Helper : calcule un hash BLAKE3 stable d'une chaîne d'audit
/// (concatène les entry_id en ordre temporel).
pub(crate) fn audit_chain_hash(entries: &[uuid::Uuid]) -> String {
    let mut hasher = blake3::Hasher::new();
    for e in entries {
        hasher.update(e.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

/// Décide le prompt final envoyé au(x) LLM(s) premium à partir du résumé produit
/// par le compresseur SLM. **Logique pure (sans I/O) → unit-testable en CI.**
///
/// Partagée par Mission A (1 LLM) et Mission B (fan-out N LLMs) : un résumé
/// dégénéré enverrait sinon du garbage à *tous* les LLMs aval.
///
/// Retourne `(prompt, fell_back)` :
/// - **Dégénéré** (résumé vide, ou contenant un placeholder de template comme
///   `<the compressed text>` / `<source text>` que le modèle 1B recrache sur les
///   inputs longs) → on retombe sur le prompt **brut**, qui préserve la
///   question ; `fell_back = true`. Qualité garantie, économie nulle sur cet
///   appel — jamais de garbage envoyé au LLM premium.
/// - **Valide** → on encadre le résumé d'une consigne (réponse brève, ne jamais
///   demander de clarification) pour que le LLM réponde toujours, même à un
///   résumé terse ; `fell_back = false`.
///
/// Le bug ayant été découvert uniquement en live (cf.
/// `live_mission_a_long_input_baseline_vs_chewed`), `tests` ci-dessous verrouille
/// la **détection** en CI sans infra SLM.
pub(crate) fn final_prompt_from_compression(
    summary: &str,
    raw_user_text: &str,
) -> (String, bool) {
    let s = summary.trim();
    let degenerate = s.is_empty()
        || s.contains("<the compressed text>")
        || s.contains("<source text>");
    if degenerate {
        return (raw_user_text.to_string(), true);
    }
    // **R5 — préambule TERSE (~50 t, contre ~140 t auparavant).** Le surcoût
    // d'encadrement était la 1re cause d'annulation des savings (un préambule de
    // 140 t ajouté à chaque appel dépasse à lui seul la plupart des prompts). On
    // condense en gardant les 4 garde-fous essentiels validés en live : réponse
    // DIRECTE, BRÈVE (une phrase), ne pas signaler un texte « incomplet », et ne
    // jamais demander de clarification.
    let framed = format!(
        "Demande condensée par un pré-traitement (question possiblement \
         implicite). Réponds DIRECTEMENT en une phrase BRÈVE, sans signaler que \
         le texte serait incomplet et sans demander de clarification :\n\n{s}"
    );
    (framed, false)
}

/// **Levier 1** — cible de compression proportionnelle à la longueur d'entrée.
///
/// Un `target_words` fixe (30) sur-comprime les longs documents : 30 mots de
/// résumé pour 2000 tokens d'entrée perd l'essentiel. On vise ~1 mot de résumé
/// pour 8 tokens d'entrée, borné à `[30, 500]` (bornes du schéma
/// `CompressorInput`). Les inputs courts gardent ~30 mots (comportement
/// historique préservé).
pub(crate) fn proportional_target_words(text: &str) -> u32 {
    (crate::metrics::ChewingMetrics::estimate_tokens(text) / 8).clamp(30, 500)
}

/// **Levier 2 (R5) — Plancher de compression (en tokens).**
///
/// En dessous de ce seuil, comprimer ne peut JAMAIS être rentable : le prompt
/// final envoyé au LLM = préambule d'encadrement
/// ([`final_prompt_from_compression`], ~50 t) + résumé (≤ 125 t, le cap de la
/// grammaire `compressor_v1.gbnf` est de 500 caractères). Comprimer une entrée
/// plus petite que ce coût plancher **gonfle** le prompt → savings négatifs
/// (observé en live).
///
/// **Invariant clé** : `COMPRESS_MIN_TOKENS` est fixé STRICTEMENT au-dessus du
/// coût max du prompt encadré (préambule + résumé cappé ≈ 175 t). Donc dès que
/// le compresseur tourne, `final_prompt` est forcément plus court que le brut ⇒
/// **savings garantis ≥ 0** (positifs, ou nuls si fallback brut sur résumé
/// dégénéré), JAMAIS négatifs. (Verrouillé par le test
/// `compress_gate_guarantees_non_negative_savings`.)
///
/// Remplace l'ancien gate `user_text.len() > 500` — qui comptait les **bytes**
/// (incohérent avec la métrique en **chars**) ET dont le seuil (~125 t) tombait
/// sous le coût du préambule, laissant passer des prompts qui ne pouvaient
/// qu'expanser.
pub(crate) const COMPRESS_MIN_TOKENS: u32 = 250;

/// `true` ssi l'entrée brute est assez longue pour que la compression puisse
/// être rentable (cf. [`COMPRESS_MIN_TOKENS`]). Sous le seuil, le compresseur
/// est skippé → `final_prompt = user_text` (savings 0, pas négatifs) + on évite
/// la latence SLM inutile.
pub(crate) fn should_compress(raw_user_text: &str) -> bool {
    crate::metrics::ChewingMetrics::estimate_tokens(raw_user_text) >= COMPRESS_MIN_TOKENS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_empty_is_blake3_default() {
        let h = audit_chain_hash(&[]);
        assert_eq!(h.len(), 64);
        assert_eq!(h, blake3::Hasher::new().finalize().to_hex().to_string());
    }

    #[test]
    fn hash_is_deterministic() {
        let e1 = uuid::Uuid::now_v7();
        let e2 = uuid::Uuid::now_v7();
        let h1 = audit_chain_hash(&[e1, e2]);
        let h2 = audit_chain_hash(&[e1, e2]);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_changes_when_order_changes() {
        let e1 = uuid::Uuid::now_v7();
        let e2 = uuid::Uuid::now_v7();
        let h1 = audit_chain_hash(&[e1, e2]);
        let h2 = audit_chain_hash(&[e2, e1]);
        assert_ne!(h1, h2);
    }

    // ── Garde anti-garbage du compresseur (partagé Mission A + B) ──────────
    // Verrouille en CI les modes d'échec découverts en live (placeholder echo,
    // résumé vide), sans aucune infra SLM.

    const RAW: &str =
        "Long contexte… quelle est la capitale de la France ? Réponds en une phrase.";

    #[test]
    fn compression_placeholder_echo_falls_back_to_raw_prompt() {
        // Mode d'échec exact observé en live : le SLM recrache le placeholder de
        // son template au lieu de comprimer → les LLMs aval recevaient du garbage.
        let (prompt, fell_back) = final_prompt_from_compression("<the compressed text>", RAW);
        assert!(fell_back, "un placeholder doit déclencher le fallback");
        assert_eq!(prompt, RAW, "on doit retomber EXACTEMENT sur le prompt brut");
    }

    #[test]
    fn compression_placeholder_embedded_in_sentence_also_caught() {
        let (prompt, fell_back) =
            final_prompt_from_compression("Résumé: <the compressed text> fin.", RAW);
        assert!(fell_back);
        assert_eq!(prompt, RAW);
    }

    #[test]
    fn compression_source_text_placeholder_caught() {
        let (_p, fell_back) = final_prompt_from_compression("blah <source text> blah", RAW);
        assert!(fell_back);
    }

    #[test]
    fn compression_empty_or_whitespace_summary_falls_back() {
        for s in ["", "   ", "\n\t "] {
            let (prompt, fell_back) = final_prompt_from_compression(s, RAW);
            assert!(fell_back, "résumé vide/blanc doit fallback (cas: {s:?})");
            assert_eq!(prompt, RAW);
        }
    }

    #[test]
    fn compression_valid_summary_is_framed_not_raw() {
        let summary = "Paris est la capitale de la France, siège des institutions.";
        let (prompt, fell_back) = final_prompt_from_compression(summary, RAW);
        assert!(!fell_back, "un résumé valide ne doit PAS fallback");
        assert_ne!(prompt, RAW, "le prompt doit être encadré, pas le brut");
        assert!(
            prompt.contains(summary),
            "le résumé doit être inclus dans l'encadrement"
        );
        // L'encadrement doit imposer la brièveté + interdire la clarification
        // (les deux fixes coût/qualité de la session).
        assert!(prompt.contains("BRÈVE"), "consigne de brièveté absente");
        assert!(
            prompt.contains("clarification"),
            "consigne anti-clarification absente"
        );
    }

    #[test]
    fn compression_summary_is_trimmed_before_framing() {
        let (prompt, fell_back) = final_prompt_from_compression("  Paris.  ", RAW);
        assert!(!fell_back);
        assert!(prompt.contains("Paris."), "le résumé trimmé doit apparaître");
    }

    #[test]
    fn proportional_target_words_scales_and_clamps() {
        // Court → plancher 30.
        assert_eq!(proportional_target_words("texte court"), 30);
        // 8000 chars ≈ 2000 tokens → 2000/8 = 250.
        assert_eq!(proportional_target_words(&"x".repeat(8000)), 250);
        // Très long → plafond 500.
        assert_eq!(proportional_target_words(&"x".repeat(80_000)), 500);
    }

    // ── R5 — corrections compression (préambule terse + gate sur tokens) ────

    use crate::metrics::ChewingMetrics;

    /// **Corr.1 — lock anti-régression du préambule.** L'ancien encadrement
    /// faisait ~140 t et annulait les savings. Pour un résumé d'1 caractère, le
    /// prompt encadré total doit rester terse (≤ 70 t). Si quelqu'un rallonge le
    /// préambule, ce test casse.
    #[test]
    fn framing_overhead_stays_terse() {
        let (framed, fell_back) = final_prompt_from_compression("x", RAW);
        assert!(!fell_back);
        let overhead = ChewingMetrics::estimate_tokens(&framed);
        assert!(overhead <= 70, "préambule trop long: {overhead} t (doit rester terse)");
    }

    /// **Corr.2 — le gate filtre bien sur le plancher en tokens.**
    #[test]
    fn should_compress_gates_on_token_floor() {
        assert!(!should_compress(""), "vide → pas de compression");
        // 200 chars ≈ 50 t < 250 → skip.
        assert!(!should_compress(&"x".repeat(200)));
        // Au plancher exact (250 t = 1000 chars) → compresse.
        assert!(should_compress(&"x".repeat(COMPRESS_MIN_TOKENS as usize * 4)));
        // Bien au-dessus → compresse.
        assert!(should_compress(&"x".repeat(4000)));
    }

    /// **Corr.1 × Corr.2 — INVARIANT cardinal** : le plancher dépasse STRICTEMENT
    /// le coût max du prompt encadré (préambule + résumé cappé à 500 car). Donc
    /// dès que le compresseur tourne (`raw ≥ COMPRESS_MIN_TOKENS`), le final est
    /// forcément plus court que le brut ⇒ savings JAMAIS négatifs. Ce test relie
    /// les deux corrections : si l'une régresse (préambule rallongé OU plancher
    /// abaissé), il casse.
    #[test]
    fn compress_gate_guarantees_non_negative_savings() {
        // Résumé de taille MAX autorisée par la grammaire (500 car), encadré.
        let (framed_max, _) = final_prompt_from_compression(&"x".repeat(500), "");
        let framed_max_tokens = ChewingMetrics::estimate_tokens(&framed_max);
        assert!(
            COMPRESS_MIN_TOKENS > framed_max_tokens,
            "plancher ({COMPRESS_MIN_TOKENS} t) doit dépasser le coût max du prompt \
             encadré ({framed_max_tokens} t) → savings ≥ 0 garantis"
        );
    }
}
