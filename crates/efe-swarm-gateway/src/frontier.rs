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

use efe_coordinator::{
    ClaudeProvider, Coordinator, GeminiProvider, GptProvider, PolicyRegistry, ProviderRegistry, TenantPolicies,
};
use uuid::Uuid;

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

    // **M3 (§2)** : charge l'allowlist d'egress par tenant, indispensable au gate fail-closed
    // (`egress_gate`). SANS elle, `coord.policies.get(tenant)` = None ⟹ **403 pour TOUT tenant**
    // (le frontière est le chemin NON-souverain : `None ⟹ DENY`). Absente ⟹ registre vide ⟹
    // fail-closed total — position honnête par défaut.
    let policies = policy_registry_from_env();

    // `build()` n'échoue que sur une config invalide (`max_concurrency == 0`) ; le défaut est
    // sain, donc `ok()` ne masque aucun cas atteignable ici.
    Coordinator::builder().registry(registry).policies(policies).build().ok().map(Arc::new)
}

/// Charge l'**allowlist d'egress par tenant** depuis `EFE_LLM_TENANT_ALLOWLIST` (côté binaire).
/// Var absente ⟹ registre **vide** (`fail-closed` total, §2). Délègue au parseur pur pour être
/// testable sans muter l'env du process.
fn policy_registry_from_env() -> PolicyRegistry {
    std::env::var("EFE_LLM_TENANT_ALLOWLIST").map(|raw| policy_registry_from_str(&raw)).unwrap_or_default()
}

/// Parseur **pur** de l'allowlist. Format : `"<tenant_uuid>=claude,gpt;<uuid2>=gemini"`.
///
/// **Contrats de correction (§2)** : (1) les identifiants LLM DOIVENT égaler ceux enregistrés
/// dans le `ProviderRegistry` (`claude`/`gpt`/`gemini`) — l'étage 2 ⊆ registre du gate rejette
/// sinon en 400 ; (2) le tenant système = UUID `00…001` ; (3) une entrée malformée (pas de `=`,
/// UUID invalide) est **ignorée** (jamais fatale), pas transformée en policy fantôme.
fn policy_registry_from_str(raw: &str) -> PolicyRegistry {
    let mut reg = PolicyRegistry::new();
    for entry in raw.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let Some((tid_s, llms)) = entry.split_once('=') else { continue };
        let Ok(tid) = Uuid::parse_str(tid_s.trim()) else { continue };
        let allowed: Vec<String> =
            llms.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
        reg.insert(TenantPolicies { allowed_llms: Some(allowed), ..TenantPolicies::open(tid) });
    }
    reg
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
    /// Le parseur d'allowlist (§2) : entrées valides ⟹ policy par tenant keyée sur l'UUID nu ;
    /// entrées malformées ignorées ; chaîne vide ⟹ registre vide (fail-closed). Pur → race-free.
    #[test]
    fn policy_registry_from_str_parses_and_skips_garbage() {
        let sys = efe_nano_sdk::TenantId::system().0;
        let reg = policy_registry_from_str(&format!("{sys}=claude, gpt ; garbage-sans-egal ; pas-un-uuid=x ; "));
        let pol = reg.get(&sys).expect("policy système présente");
        assert_eq!(
            pol.allowed_llms.as_deref(),
            Some(&["claude".to_string(), "gpt".to_string()][..]),
            "les 2 LLM autorisés sont parsés (espaces trimés)"
        );
        assert!(reg.get(&Uuid::nil()).is_none(), "les entrées malformées ne créent aucune policy fantôme");
        assert!(policy_registry_from_str("").get(&sys).is_none(), "chaîne vide ⟹ fail-closed total");
    }

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
