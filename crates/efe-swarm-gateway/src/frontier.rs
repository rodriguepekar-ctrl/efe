// SPDX-License-Identifier: UNLICENSED
//! **Couche 2 — coordination frontière (Claude / Gemini / GPT).**
//!
//! Construit le [`Coordinator`] server-side à partir des clés provider présentes dans l'env
//! (`ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GOOGLE_AI_KEY`). C'est la **couture
//! d'intégration** de la Couche 2 : les futures routes `/v1/llm/*` (M3) exécuteront leurs
//! missions (`vote` / `relay` / `debate` / `specialist` / `cascade`) sur ce coordinator,
//! et la **capture de provenance** (M1) décorera chaque provider **ici même** — un
//! `LlmProvenanceRecord` scellé par appel — sans changer la signature publique ci-dessous.
//!
//! **M0+M1** : on enregistre les providers frontière **réels** (best-effort — un provider
//! dont la clé manque est simplement absent), **chacun enveloppé** du décorateur de provenance
//! [`crate::mission::ProvenanceProvider`] (M1). Aucune route n'est encore montée (M3) : ce module
//! assemble le coordinator décoré, gouverné au runtime par `EFE_LLM_FRONTIER_ENABLED` côté binaire.
//!
//! **Souveraineté** : aucune clé n'est loggée ni copiée hors du provider. Les constructeurs
//! `from_env` du coordinator lisent la clé au moment de bâtir le client HTTP ; cet état
//! n'en détient jamais de copie et n'en scelle jamais dans un bundle. Le chemin frontière
//! est le chemin **haute-capacité NON souverain** (prompt + latent sortent vers
//! Anthropic/OpenAI/Google) — à opposer proprement au chemin méta-essaim.

use std::sync::Arc;

use efe_coordinator::{ClaudeProvider, Coordinator, GeminiProvider, GptProvider, ProviderRegistry};

/// Construit un [`Coordinator`] frontière depuis l'env — **best-effort**.
///
/// Enregistre chaque provider dont la clé d'API est présente (`ANTHROPIC_API_KEY`,
/// `OPENAI_API_KEY`, `GOOGLE_AI_KEY`). Renvoie `None` si **aucun** provider n'a pu être
/// bâti (aucune clé) : l'appelant laisse alors la frontière **non câblée** plutôt que
/// d'exposer un coordinator vide. Ne panique jamais — une clé absente ⇒ provider omis
/// (les `from_env` renvoient une `Err` silencieuse, pas un panic).
///
/// **M1 (fait)** : chaque provider est enveloppé d'un [`crate::mission::ProvenanceProvider`]
/// qui scelle l'appel (endpoint **observé** → `R4RemoteChannel` distant, hachés blake3,
/// échecs compris) dans le puits de mission actif — capture `no-op` hors d'un scope de mission.
#[must_use]
pub fn coordinator_from_env() -> Option<Arc<Coordinator>> {
    let registry = ProviderRegistry::new();
    let mut registered = 0usize;

    // Best-effort : un provider dont la clé manque est omis (from_env → Err), jamais fatal.
    if let Ok(p) = ClaudeProvider::from_env() {
        registry.register(crate::mission::ProvenanceProvider::wrap(Arc::new(p)));
        registered += 1;
    }
    if let Ok(p) = GptProvider::from_env() {
        registry.register(crate::mission::ProvenanceProvider::wrap(Arc::new(p)));
        registered += 1;
    }
    if let Ok(p) = GeminiProvider::from_env() {
        registry.register(crate::mission::ProvenanceProvider::wrap(Arc::new(p)));
        registered += 1;
    }

    if registered == 0 {
        return None;
    }

    // `build()` n'échoue que sur une config invalide (`max_concurrency == 0`) ; le défaut est
    // sain, donc `ok()` ne masque aucun cas atteignable ici.
    Coordinator::builder().registry(registry).build().ok().map(Arc::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sans aucune clé provider dans l'env, la construction renvoie `None` (frontière non
    /// câblée) plutôt qu'un coordinator vide — le contrat sur lequel `main` s'appuie pour
    /// n'activer la frontière que si au moins un modèle est réellement joignable.
    ///
    /// Note : test **non déterministe si l'env porte déjà une clé** ; il n'affirme donc que
    /// l'invariant faible « pas de panic, résultat cohérent avec le nombre de clés vues ».
    #[test]
    fn build_is_infallible_and_shapes_with_env() {
        let has_any_key = std::env::var("ANTHROPIC_API_KEY").is_ok()
            || std::env::var("OPENAI_API_KEY").is_ok()
            || std::env::var("GOOGLE_AI_KEY").is_ok();
        match coordinator_from_env() {
            Some(_) => assert!(has_any_key, "un coordinator n'est bâti que si ≥ 1 clé est présente"),
            None => assert!(!has_any_key, "aucune clé → None (frontière non câblée)"),
        }
    }
}
