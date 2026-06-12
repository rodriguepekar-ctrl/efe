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
#[derive(Default)]
pub enum ClientMode {
    /// **Keyless** : agents déterministes en process (flotte Mode A / `BuiltinLlmClient`).
    /// Aucune clé, aucune dépendance réseau. Le mode démo/dev/CI.
    #[default]
    Keyless,
    /// **Self-host** : modèle local (`llama-server`/Ollama). **Aucune clé** — souverain,
    /// on-prem, air-gappable. Le mode « privacy » phare.
    SelfHost { endpoint: String, model: String },
    /// **Network** : routeur de modèles frontières (M21). Les clés sont gérées
    /// **server-side** (jamais ici).
    Network { provider: String },
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

// ─────────────────────────────────────────────────────────────────────────────
//  WidthSpec — la largeur d'exécution (ADR-014, architecture §3)
// ─────────────────────────────────────────────────────────────────────────────

/// **Largeur d'exécution** d'un run : combien d'appels d'agents sont en vol
/// simultanément dans une vague. C'est une **politique d'exécution, jamais une
/// sémantique** (I-CP1) : mêmes cellules, mêmes records, même racine quelle que soit
/// la largeur — seules les durées changent.
///
/// Résolution (`Auto`) **par mode** :
/// - **Mode A** (keyless, nano-agents CPU purs) : les cœurs mesurés
///   (`std::thread::available_parallelism()`, plancher 2) ;
/// - **Mode C** (self-host) : la capacité **déclarée** du backend
///   (`suggested_width()`, défaut conservateur 2 : pousser plus large vers un backend
///   qui sérialise n'achète que des files).
///
/// Précédence : builder (`Fixed`) > Auto. **Le SDK ne lit jamais l'environnement**
/// (garde-fou G1, keyless par construction — `keyless_guardrails.rs`) : l'override
/// opérateur `EFE_SWARM_WIDTH` est lu **côté passerelle** (le binaire) et injecté ici
/// via `Fixed`, exactement comme `EFE_SWARM_ROUTER_ENDPOINT`. La largeur effective
/// d'une vague est toujours `min(taille de vague, largeur)` ≥ 1 (I-CP3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WidthSpec {
    /// `min(vague, P_eff)` — P_eff résolu par mode (cœurs / backend). Le défaut.
    #[default]
    Auto,
    /// Largeur imposée (normalisée ≥ 1).
    Fixed(usize),
}

impl WidthSpec {
    /// Résout la largeur : spec > Auto-par-mode. **Pure** (aucune lecture d'env — G1) ;
    /// `backend_width` est la capacité déclarée du backend (`None` en Mode A).
    pub(crate) fn resolve(&self, mode: &ClientMode, backend_width: Option<usize>) -> usize {
        match self {
            WidthSpec::Fixed(w) => (*w).max(1), // I-CP3 : jamais 0 voie
            WidthSpec::Auto => match mode {
                // Mode A : la loi des cœurs (plancher 2 — une machine mono-cœur
                // exotique garde un minimum de recouvrement).
                ClientMode::Keyless => cpu_parallelism(),
                // Mode C / Network-traduit : la loi du backend (déclaratif, conservateur).
                _ => backend_width.unwrap_or(2).max(1),
            },
        }
    }
}

/// Parallélisme CPU mesuré (plancher 2). La SEULE lecture machine du SDK — côté
/// **exécution** uniquement : aucune grandeur sémantique n'en dérive (architecture §2.3).
pub(crate) fn cpu_parallelism() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8).max(2)
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

    // ── WidthSpec (I-CP3 : résolution totale — PURE, le SDK ne lit jamais l'env : G1) ──
    #[test]
    fn width_resolution_clamps_all_inputs() {
        let sh = ClientMode::SelfHost { endpoint: "x".into(), model: "m".into() };
        // Fixed normalisé ≥ 1 (jamais 0 voie).
        assert_eq!(WidthSpec::Fixed(0).resolve(&ClientMode::Keyless, None), 1);
        assert_eq!(WidthSpec::Fixed(6).resolve(&ClientMode::Keyless, None), 6);
        // Auto Mode A : les cœurs mesurés (plancher 2).
        let auto_a = WidthSpec::Auto.resolve(&ClientMode::Keyless, None);
        assert!(auto_a >= 2, "Auto Mode A ≥ 2, eu {auto_a}");
        assert_eq!(auto_a, cpu_parallelism());
        // Auto Mode C : la capacité déclarée du backend ; défaut conservateur 2.
        assert_eq!(WidthSpec::Auto.resolve(&sh, Some(4)), 4);
        assert_eq!(WidthSpec::Auto.resolve(&sh, None), 2);
        assert_eq!(WidthSpec::Auto.resolve(&sh, Some(0)), 1, "backend 0 → plancher 1");
    }
}
