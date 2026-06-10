// SPDX-License-Identifier: UNLICENSED
//! **Mapping provider→modèle (Inc.4.b)** — le protocole concret du mode `Network`.
//!
//! Le mode `Network { provider }` désigne une **famille** de provider (`"anthropic"`,
//! `"openai"`…). Le routeur frontière (`efe-llm-router`, OpenAI-compatible server-side) attend
//! un **identifiant de modèle** concret (`"anthropic/claude-3-5-sonnet-latest"`…). Cette table
//! traduit l'un en l'autre, **côté passerelle**, avant d'émettre l'appel.
//!
//! **Souveraineté / clés** : un identifiant de modèle **n'est pas un secret** — c'est une
//! politique de routage. La passerelle n'a JAMAIS de clé provider ; le routeur les détient
//! server-side (env/Vault). Cette table est donc sûre à porter ici et configurable au déploiement.
//!
//! **Règles de résolution** (déterministes) :
//! 1. `family:model` ou `family/model` **explicite** (déjà qualifié) ⇒ **passthrough** —
//!    l'appelant a tranché, on ne touche à rien.
//! 2. **famille connue** (table par défaut, surchargée par l'env) ⇒ identifiant de modèle.
//! 3. **famille inconnue** ⇒ **passthrough** (le routeur tranche, ou renvoie une erreur claire).
//!
//! Les défauts privilégient les **alias `-latest`** (résistants à l'obsolescence) et la forme
//! `provider/model` (convention des routeurs OpenAI-compat type LiteLLM), qui **évite** le
//! piège du préfixe `openai:` (lequel forcerait le moteur OpenAI-local côté SDK).

use std::collections::HashMap;

/// Table par défaut famille→modèle. Surchargeable au déploiement (`EFE_SWARM_ROUTER_MODELS`).
fn default_table() -> HashMap<String, String> {
    [
        ("anthropic", "anthropic/claude-3-5-sonnet-latest"),
        ("openai", "openai/gpt-4o"),
        ("mistral", "mistral/mistral-large-latest"),
        ("google", "vertex_ai/gemini-1.5-pro"),
        ("vertex", "vertex_ai/gemini-1.5-pro"),
        ("gemini", "vertex_ai/gemini-1.5-pro"),
        ("groq", "groq/llama-3.3-70b-versatile"),
        ("cohere", "cohere/command-r-plus"),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
    .collect()
}

/// Table de routage **provider→modèle** (par défaut + surcharges). Pas de secret : noms de modèles.
#[derive(Debug, Clone)]
pub struct RouterMap {
    map: HashMap<String, String>,
}

impl Default for RouterMap {
    fn default() -> Self {
        Self { map: default_table() }
    }
}

impl RouterMap {
    /// Table par défaut (familles courantes).
    pub fn new() -> Self {
        Self::default()
    }

    /// Table **vide** (tout en passthrough) — utile en test, ou pour un déploiement qui
    /// impose des modèles explicites.
    pub fn empty() -> Self {
        Self { map: HashMap::new() }
    }

    /// Ajoute/écrase une entrée famille→modèle (la famille est insensible à la casse).
    pub fn with(mut self, family: impl Into<String>, model: impl Into<String>) -> Self {
        self.map.insert(family.into().to_ascii_lowercase(), model.into());
        self
    }

    /// **Résout** un provider en identifiant de modèle pour le routeur (cf. règles du module).
    pub fn resolve(&self, provider: &str) -> String {
        let p = provider.trim();
        if p.contains(':') || p.contains('/') {
            return p.to_string(); // déjà qualifié → passthrough
        }
        match self.map.get(&p.to_ascii_lowercase()) {
            Some(model) => model.clone(),
            None => p.to_string(), // famille inconnue → passthrough (le routeur tranche)
        }
    }

    /// Parse une chaîne d'environnement `famille=modèle,famille=modèle` (lue **côté binaire**,
    /// jamais dans la lib), **fusionnée sur les défauts**. Les entrées mal formées sont ignorées.
    pub fn from_env_str(s: &str) -> Self {
        let mut m = Self::default();
        for pair in s.split(',') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            if let Some((k, v)) = pair.split_once('=') {
                let (k, v) = (k.trim(), v.trim());
                if !k.is_empty() && !v.is_empty() {
                    m.map.insert(k.to_ascii_lowercase(), v.to_string());
                }
            }
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_family_maps_to_default_model() {
        let m = RouterMap::new();
        assert_eq!(m.resolve("anthropic"), "anthropic/claude-3-5-sonnet-latest");
        assert_eq!(m.resolve("openai"), "openai/gpt-4o");
        // insensible à la casse + espaces tolérés.
        assert_eq!(m.resolve("  Anthropic "), "anthropic/claude-3-5-sonnet-latest");
    }

    #[test]
    fn explicit_qualifier_is_passthrough() {
        let m = RouterMap::new();
        // l'appelant a déjà tranché un modèle concret → on ne ré-écrit pas.
        assert_eq!(m.resolve("anthropic:claude-3-opus"), "anthropic:claude-3-opus");
        assert_eq!(m.resolve("openai/gpt-4o-mini"), "openai/gpt-4o-mini");
    }

    #[test]
    fn unknown_family_is_passthrough() {
        let m = RouterMap::new();
        assert_eq!(m.resolve("acme-llm"), "acme-llm");
    }

    #[test]
    fn override_wins_over_default() {
        let m = RouterMap::new().with("anthropic", "anthropic/claude-3-5-haiku-latest");
        assert_eq!(m.resolve("anthropic"), "anthropic/claude-3-5-haiku-latest");
    }

    #[test]
    fn from_env_str_merges_over_defaults() {
        let m = RouterMap::from_env_str("openai=openai/gpt-4o-mini, acme=acme/turbo , bad-entry,=skip");
        assert_eq!(m.resolve("openai"), "openai/gpt-4o-mini", "surcharge");
        assert_eq!(m.resolve("acme"), "acme/turbo", "ajout");
        assert_eq!(m.resolve("anthropic"), "anthropic/claude-3-5-sonnet-latest", "défaut conservé");
        assert_eq!(m.resolve("bad-entry"), "bad-entry", "entrée sans '=' ignorée → passthrough");
    }

    #[test]
    fn empty_map_is_all_passthrough() {
        let m = RouterMap::empty();
        assert_eq!(m.resolve("anthropic"), "anthropic");
    }
}
