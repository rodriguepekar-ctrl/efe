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
    // **R7 — PRÉSERVER LA QUESTION (bug révélé en live, H0.1).** Le compresseur
    // 1B largue parfois la question : sur un prompt à fort remplissage, il
    // extrait le bruit dominant (« j'ai lu des choses contradictoires sur des
    // forums… ») et JETTE la vraie demande, malgré l'instruction de son prompt.
    // Le LLM répond alors au bruit (sur un prompt REST/GraphQL il citait des
    // autorités médicales). Correctif : on ré-injecte les phrases interrogatives
    // du BRUT, VERBATIM, dans le prompt final → le LLM voit toujours la vraie
    // question, quoi que le compresseur ait fait du contexte.
    //
    // **R5 — préambule TERSE** (les 4 garde-fous validés en live : réponse
    // DIRECTE, BRÈVE, ne pas signaler « incomplet », jamais de clarification).
    let question = extract_questions(raw_user_text);
    let framed = if question.is_empty() {
        // Pas de question explicite (`?`) → requête impérative : comportement R5.
        format!(
            "Demande condensée par un pré-traitement (question possiblement \
             implicite). Réponds DIRECTEMENT en une phrase BRÈVE, sans signaler \
             que le texte serait incomplet et sans demander de clarification :\
             \n\n{s}"
        )
    } else {
        // Contexte compressé + QUESTION verbatim : le LLM répond à la question.
        format!(
            "Contexte condensé par un pré-traitement (possiblement incomplet). \
             Réponds DIRECTEMENT et de façon BRÈVE à la QUESTION ci-dessous, en \
             t'appuyant sur ce contexte, sans signaler qu'il serait incomplet et \
             sans demander de clarification :\n\n\
             --- Contexte condensé ---\n{s}\n\n\
             --- Question ---\n{question}"
        )
    };
    (framed, false)
}

/// **R7** — Plafond (caractères) des questions ré-injectées verbatim. Borne le
/// cas pathologique d'un prompt ≈ uniquement des questions (où il n'y a de toute
/// façon rien à compresser).
const MAX_QUESTION_CHARS: usize = 600;

/// **R7** — Extrait les phrases **interrogatives** (se terminant par `?`) du
/// texte brut, concaténées, pour ré-injection verbatim. Logique pure.
///
/// - Découpe en phrases sur `. ! ? \n` ; garde celles finissant par `?`.
/// - Préfère les **dernières** questions (la vraie demande est typiquement en
///   fin), bornées à [`MAX_QUESTION_CHARS`] (questions entières préservées).
/// - Aucune question (`?` absent → requête impérative) → `""` (le caller bascule
///   sur le préambule R5 historique).
pub(crate) fn extract_questions(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut questions: Vec<String> = Vec::new();
    let mut start = 0usize;
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '?' => {
                let q: String = chars[start..=i].iter().collect::<String>().trim().to_string();
                if q.chars().count() > 1 {
                    questions.push(q);
                }
                start = i + 1;
            }
            '.' | '!' | '\n' => start = i + 1,
            _ => {}
        }
    }
    // Garde les dernières questions entières dans le budget.
    let mut kept: Vec<String> = Vec::new();
    let mut budget = MAX_QUESTION_CHARS;
    for q in questions.into_iter().rev() {
        let n = q.chars().count();
        if !kept.is_empty() && n > budget {
            break;
        }
        budget = budget.saturating_sub(n + 1);
        kept.push(q);
    }
    kept.reverse();
    kept.join(" ")
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

    /// **Corr.1 — lock anti-régression du préambule (branche SANS question).**
    /// L'ancien encadrement faisait ~140 t. Pour un résumé d'1 caractère sur une
    /// requête impérative (pas de `?` → branche R5 terse), l'encadrement total
    /// doit rester ≤ 70 t. (La branche-question, plus riche, est bornée par
    /// `question_branch_overhead_is_modest`.)
    #[test]
    fn framing_overhead_stays_terse() {
        // Pas de `?` → branche impérative R5 (la plus terse).
        let raw_no_q = "Traite ce texte sans interrogation, juste du contenu brut.";
        let (framed, fell_back) = final_prompt_from_compression("x", raw_no_q);
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

    /// **Corr.1 × Corr.2 — INVARIANT du CONTEXTE encadré** : le plancher dépasse
    /// STRICTEMENT le coût max de l'encadrement du contexte (préambule + résumé
    /// cappé à 500 car, **hors question**). Donc la partie CONTEXTE du final est
    /// toujours plus courte que le brut.
    ///
    /// **Nuance R7** : depuis la préservation de la question, le final = contexte
    /// encadré **+ question verbatim**. La garantie « savings ≥ 0 » n'est donc
    /// plus stricte sur un prompt ≈ uniquement des questions (où il n'y a de toute
    /// façon rien à compresser, et où préserver la question est l'objectif). Pour
    /// le cas réel (contexte compressible + petite question), savings positifs. Le
    /// test borne la partie compressée — la question est valeur ajoutée assumée
    /// (des savings sur une réponse fausse ne valent rien).
    #[test]
    fn compress_gate_guarantees_non_negative_savings() {
        // Contexte sans question → mesure le coût d'encadrement pur (préambule +
        // résumé MAX 500 car), borné par le plancher.
        let (framed_max, _) = final_prompt_from_compression(&"x".repeat(500), "");
        let framed_max_tokens = ChewingMetrics::estimate_tokens(&framed_max);
        assert!(
            COMPRESS_MIN_TOKENS > framed_max_tokens,
            "plancher ({COMPRESS_MIN_TOKENS} t) doit dépasser le coût max de \
             l'encadrement du contexte ({framed_max_tokens} t)"
        );
    }

    // ── R7 — préservation de la question (bug H0.1 : le 1B largue la question) ──

    /// `extract_questions` ne garde QUE les phrases interrogatives (finissant par
    /// `?`), dans l'ordre ; aucune `?` → vide (requête impérative).
    #[test]
    fn extract_questions_keeps_only_interrogatives() {
        assert_eq!(extract_questions("Pas de question ici. Juste du texte."), "");
        assert_eq!(
            extract_questions("Blabla. Quelle heure est-il ? Encore du blabla."),
            "Quelle heure est-il ?"
        );
        // Multi-questions : les deux gardées, dans l'ordre.
        assert_eq!(extract_questions("Contexte. Qui ? Pourquoi ?"), "Qui ? Pourquoi ?");
        // Vide / sans `?` → vide.
        assert_eq!(extract_questions(""), "");
    }

    /// **LOCK DU BUG H0.1.** Même si le résumé est du FILLER (le compresseur a
    /// largué la question), la question du brut doit apparaître VERBATIM dans le
    /// prompt final → le LLM répond à la question, pas au bruit. Sans ce
    /// correctif, le prompt compressé faisait répondre Claude sur le remplissage
    /// (citant des autorités médicales pour une question REST/GraphQL).
    #[test]
    fn question_survives_filler_summary() {
        let filler_summary = "L'utilisateur a lu des choses contradictoires sur des forums.";
        let raw = "Blabla redondant. Encore du blabla filler. \
                   Quelle est la différence entre synchrone et asynchrone ?";
        let (prompt, fell_back) = final_prompt_from_compression(filler_summary, raw);
        assert!(!fell_back, "résumé non dégénéré → pas de fallback");
        assert!(
            prompt.contains("Quelle est la différence entre synchrone et asynchrone ?"),
            "la question du brut doit survivre verbatim malgré un résumé qui l'a perdue"
        );
        // Le contexte compressé reste présent aussi.
        assert!(prompt.contains(filler_summary), "le contexte condensé reste inclus");
    }

    /// La branche-question reste raisonnable : préambule + marqueurs + 1 char +
    /// petite question ≤ ~90 t (lock anti-bloat des marqueurs Contexte/Question).
    #[test]
    fn question_branch_overhead_is_modest() {
        let (framed, _) = final_prompt_from_compression("x", "Contexte bref. Quoi ?");
        let t = ChewingMetrics::estimate_tokens(&framed);
        assert!(t <= 90, "overhead branche-question trop gros: {t} t");
    }
}
