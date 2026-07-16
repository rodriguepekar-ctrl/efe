// SPDX-License-Identifier: UNLICENSED
//! **Fil B.0** — quel moteur a effectivement produit une reformulation.
//!
//! Le reformulateur (traducteur dialecte-LLM → schéma canonique) peut être
//! rendu par deux moteurs : le déterministe [`super::CanonicalReformulator`]
//! (parsing pur Rust, toujours disponible) ou le vrai petit modèle
//! [`super::SlmReformulator`] (Qwen3-0.6B via llama-server, câblé en Fil B.2).
//!
//! Le point d'honnêteté central de Fil B : on ne peut revendiquer « médié par
//! un petit modèle » **que** pour les hops estampillés [`ReformulatorEngine::Slm`].
//! Une dégradation vers le déterministe (SLM absent / en panne / schéma non
//! supporté) est **toujours** estampillée [`ReformulatorEngine::SlmDegradedToRule`]
//! — jamais présentée en silence comme du SLM.

use serde::{Deserialize, Serialize};

/// Le moteur qui a produit une reformulation donnée (porté par hop dans le reçu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReformulatorEngine {
    /// Reformulateur déterministe (parsing Rust). C'est le défaut et le repli.
    Rule,
    /// Vrai petit modèle (SLM) — la reformulation a réellement transité par le
    /// modèle et sa sortie a été validée.
    Slm,
    /// Le SLM était censé traiter ce hop mais a été indisponible / a échoué /
    /// ne supporte pas le schéma : on a **dégradé** vers le déterministe. La
    /// sortie est fiable, mais ce n'est **pas** du SLM — et on le dit.
    SlmDegradedToRule,
}

impl ReformulatorEngine {
    /// Tag stable pour logs / SSE / reçu de provenance.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Rule => "RULE",
            Self::Slm => "SLM",
            Self::SlmDegradedToRule => "SLM_DEGRADED_TO_RULE",
        }
    }

    /// Vrai **uniquement** si un petit modèle a réellement produit la sortie.
    /// À utiliser pour toute revendication « médié par le SLM ».
    #[must_use]
    pub fn is_slm(self) -> bool {
        matches!(self, Self::Slm)
    }
}

/// Résultat d'une reformulation : la valeur canonique **plus** le moteur qui
/// l'a produite. Le moteur voyage avec la valeur pour que chaque hop puisse
/// être estampillé honnêtement dans le reçu (jamais un `engine_tag()` global
/// et racé).
#[derive(Debug, Clone)]
pub struct Reformulated {
    /// La forme canonique typée, sérialisée (`serde_json::Value`), identique en
    /// type à la sortie de [`super::CanonicalReformulator`] **et** du SLM.
    pub value: serde_json::Value,
    /// Le moteur effectif (voir [`ReformulatorEngine`]).
    pub engine: ReformulatorEngine,
}

impl Reformulated {
    /// Constructeur pratique.
    #[must_use]
    pub fn new(value: serde_json::Value, engine: ReformulatorEngine) -> Self {
        Self { value, engine }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_stable() {
        assert_eq!(ReformulatorEngine::Rule.tag(), "RULE");
        assert_eq!(ReformulatorEngine::Slm.tag(), "SLM");
        assert_eq!(ReformulatorEngine::SlmDegradedToRule.tag(), "SLM_DEGRADED_TO_RULE");
    }

    #[test]
    fn only_slm_is_slm() {
        assert!(ReformulatorEngine::Slm.is_slm());
        assert!(!ReformulatorEngine::Rule.is_slm());
        assert!(!ReformulatorEngine::SlmDegradedToRule.is_slm());
    }

    #[test]
    fn engine_serializes_screaming_snake() {
        assert_eq!(
            serde_json::to_string(&ReformulatorEngine::SlmDegradedToRule).unwrap(),
            "\"SLM_DEGRADED_TO_RULE\""
        );
    }
}
