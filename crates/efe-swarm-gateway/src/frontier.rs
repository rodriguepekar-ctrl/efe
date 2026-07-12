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
    ClaudeProvider, Coordinator, GeminiProvider, GptProvider, LlmProvider, PolicyRegistry, ProviderRegistry,
    TenantPolicies,
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
    let mut ids: Vec<String> = Vec::new();

    // Best-effort : un provider dont la clé manque est omis (from_env → Err), jamais fatal.
    // L'id est capturé AVANT le wrap (qui consomme le provider) — cf. log ci-dessous (§3.3).
    if let Ok(p) = ClaudeProvider::from_env() {
        ids.push(p.id());
        registry.register(crate::mission::ProvenanceProvider::wrap(Arc::new(p)));
    }
    if let Ok(p) = GptProvider::from_env() {
        ids.push(p.id());
        registry.register(crate::mission::ProvenanceProvider::wrap(Arc::new(p)));
    }
    if let Ok(p) = GeminiProvider::from_env() {
        ids.push(p.id());
        registry.register(crate::mission::ProvenanceProvider::wrap(Arc::new(p)));
    }

    if ids.is_empty() {
        return None;
    }

    // **M4 (§3.3)** : les ids enregistrés portent le MODÈLE (`claude_<model>`, `gpt_<model>`,
    // `gemini_<model>`) — l'allowlist `EFE_LLM_TENANT_ALLOWLIST` doit les citer EXACTEMENT
    // (l'étage 2 ⊆ registre du gate rejette sinon en 400). On les logge au boot pour rendre
    // les allowlists authorables sans deviner le suffixe-modèle. Aucune clé n'est loggée (ids seuls).
    eprintln!("→ providers frontière enregistrés (ids pour EFE_LLM_TENANT_ALLOWLIST) : {ids:?}");

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

/// Parseur **pur** de l'allowlist. Format :
/// `"<tenant_uuid>=<id1>,<id2>[@cap=<cents>];<uuid2>=<id3>"`.
///
/// **Contrats (§2 / §2.1 / §3.3)** :
/// - les identifiants LLM DOIVENT égaler ceux **enregistrés** — `claude_<model>` / `gpt_<model>` /
///   `gemini_<model>` (logués au boot par `coordinator_from_env`) — l'étage 2 ⊆ registre du gate
///   rejette sinon en **400** (correction du piège `ADV-5` : jadis documenté `claude`/`gpt`/`gemini`
///   nus, qui ne matchent JAMAIS le registre suffixé-modèle) ;
/// - **`@cap=<cents>`** (optionnel) pose le **plafond de coût par requête** du tenant, en **cents**
///   (stocké en microcents ×1000). Absent ⟹ pas de plafond tenant (seul le budget de requête
///   s'applique — cf. `clamp_budget`) ;
/// - tenant système = UUID `00…001` ; une entrée malformée (pas de `=`, UUID invalide) est
///   **ignorée** (jamais fatale), pas transformée en policy fantôme.
fn policy_registry_from_str(raw: &str) -> PolicyRegistry {
    let mut reg = PolicyRegistry::new();
    for entry in raw.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let Some((tid_s, rest)) = entry.split_once('=') else { continue };
        let Ok(tid) = Uuid::parse_str(tid_s.trim()) else { continue };
        // Sépare la liste des LLM d'un éventuel plafond `@cap=<cents>` (cents → microcents ×1000).
        let (llms_s, cap_microcents) = match rest.split_once("@cap=") {
            Some((llms, cap_s)) => (llms, cap_s.trim().parse::<u64>().ok().map(|cents| cents * 1000)),
            None => (rest, None),
        };
        let allowed: Vec<String> =
            llms_s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
        reg.insert(TenantPolicies {
            allowed_llms: Some(allowed),
            max_cost_microcents_per_query: cap_microcents,
            ..TenantPolicies::open(tid)
        });
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
    /// Le parseur d'allowlist (§2/§2.1) : ids **suffixés-modèle** + plafond `@cap` optionnel ;
    /// entrées malformées ignorées ; chaîne vide ⟹ fail-closed. Pur → race-free.
    #[test]
    fn policy_registry_from_str_parses_allowlist_and_cap() {
        let sys = efe_nano_sdk::TenantId::system().0;
        let reg = policy_registry_from_str(&format!(
            "{sys}=claude_sonnet, gpt_4o @cap=50000 ; garbage-sans-egal ; pas-un-uuid=x ; "
        ));
        let pol = reg.get(&sys).expect("policy système présente");
        assert_eq!(
            pol.allowed_llms.as_deref(),
            Some(&["claude_sonnet".to_string(), "gpt_4o".to_string()][..]),
            "les 2 ids suffixés-modèle sont parsés (espaces trimés)"
        );
        assert_eq!(pol.max_cost_microcents_per_query, Some(50_000 * 1000), "@cap=50000 cents = 50M microcents");
        let no_cap = policy_registry_from_str(&format!("{sys}=claude_sonnet"));
        assert_eq!(no_cap.get(&sys).unwrap().max_cost_microcents_per_query, None, "sans @cap ⟹ pas de plafond tenant");
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
