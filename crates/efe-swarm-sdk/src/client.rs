// SPDX-License-Identifier: UNLICENSED
//! `ClientMode` — keyless / self-host / réseau (le pattern `build_client`).
//!
//! **Sécurité (règle absolue)** : ce module **ne saisit, ne lit, ne stocke aucune clé**.
//! `Keyless`/`SelfHost` sont keyless par construction ; `Network` délègue les clés au
//! routeur **server-side** (env/Vault). Le binaire ne contient aucun secret.
//!
//! En Inc.1, `ClientMode` pilote le **modèle de prix** du budget (cents par appel
//! d'agent) et le **mode de déploiement** rapporté dans `SwarmRun`. Le câblage d'un
//! `LlmClient` réel pour des agents **Mode C** (déjà prouvé dans
//! `efe-swarm-provenance/examples/swarm_all_phases_real.rs` avec Ollama local) est la
//! tranche suivante ; le cœur Inc.1 exécute la flotte **Mode A** (déterministe, keyless).

use serde::Serialize;

/// Où s'exécutent les agents LLM (le cas échéant) — et le prix associé.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ClientMode {
    /// **Keyless** : agents déterministes en process (flotte Mode A / `BuiltinLlmClient`).
    /// Aucune clé, aucune dépendance réseau. Le mode démo/dev/CI.
    Keyless,
    /// **Self-host** : modèle local (`llama-server`/Ollama). **Aucune clé** — souverain,
    /// on-prem, air-gappable. Le mode « privacy » phare.
    SelfHost { endpoint: String, model: String },
    /// **Network** : routeur de modèles frontières (M21). Les clés sont gérées
    /// **server-side** (jamais ici).
    Network { provider: String },
}

impl Default for ClientMode {
    fn default() -> Self {
        ClientMode::Keyless
    }
}

impl ClientMode {
    /// Prix NOMINAL par appel d'agent (cents) pour la gouvernance de budget.
    /// - Keyless : compute local déterministe (nominal, faible).
    /// - Self-host : local (gratuit) → 0¢.
    /// - Network : modèle hébergé (coût réel via le routeur ; valeur indicative ici).
    pub fn price_per_call_cents(&self) -> u32 {
        match self {
            ClientMode::Keyless => 1,
            ClientMode::SelfHost { .. } => 0,
            ClientMode::Network { .. } => 5,
        }
    }

    /// Étiquette courte (pour la ventilation budget + l'observabilité).
    pub fn label(&self) -> String {
        match self {
            ClientMode::Keyless => "keyless".into(),
            ClientMode::SelfHost { model, .. } => format!("selfhost:{model}"),
            ClientMode::Network { provider } => format!("network:{provider}"),
        }
    }

    /// Le mode est-il **keyless** (aucune clé requise) ?
    pub fn is_keyless(&self) -> bool {
        !matches!(self, ClientMode::Network { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_model_per_mode() {
        assert_eq!(ClientMode::Keyless.price_per_call_cents(), 1);
        assert_eq!(ClientMode::SelfHost { endpoint: "x".into(), model: "gemma3:12b".into() }.price_per_call_cents(), 0);
        assert_eq!(ClientMode::Network { provider: "anthropic".into() }.price_per_call_cents(), 5);
    }

    #[test]
    fn keyless_modes_need_no_key() {
        assert!(ClientMode::Keyless.is_keyless());
        assert!(ClientMode::SelfHost { endpoint: "x".into(), model: "m".into() }.is_keyless());
        assert!(!ClientMode::Network { provider: "p".into() }.is_keyless());
    }
}
