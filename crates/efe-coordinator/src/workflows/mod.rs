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
    let framed = format!(
        "Le texte ci-dessous est une demande d'un utilisateur, condensée par un \
         pré-traitement (le contexte a été résumé, la question peut être \
         implicite). Tu DOIS y répondre. Déduis la question la plus probable à \
         partir du contenu et donne DIRECTEMENT la réponse, de façon BRÈVE (une \
         seule phrase courte), sans préambule ni développement — une réponse \
         verbeuse annulerait l'économie de la compression. N'indique JAMAIS que \
         le texte est vide, coupé ou incomplet, et ne demande JAMAIS de \
         clarification : réponds avec l'information disponible.\n\n\
         --- Demande condensée ---\n{s}"
    );
    (framed, false)
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
}
