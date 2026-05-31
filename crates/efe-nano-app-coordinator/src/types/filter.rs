// SPDX-License-Identifier: UNLICENSED
//! Types I/O du robot **Filtreur** (bge-m3 via fastembed-rs).
//!
//! Robot ONNX (pas de génération) : pour un message entrant, décide
//! quel(s) LLM agent(s) doivent en être notifiés via NATS, en
//! fonction de la pertinence sémantique et du budget de contexte.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use validator::Validate;

/// Identifiant d'un agent LLM (ex: `claude_opus_4`, `gpt4_turbo`, ...).
pub type AgentRef = String;

/// Vecteur de capacités sémantiques d'un agent (dim 1024, bge-m3).
///
/// Représente "ce que cet agent sait bien faire" en projection dans
/// l'espace bge-m3.  Calculé à la déclaration de l'agent en concaténant
/// ses descriptions de capacités puis en l'embeddant.
pub type CapabilityVector = Vec<f32>;

/// Entrée du robot Filtreur.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct FilterInput {
    /// Résumé court du message à router (idéalement déjà compressé par L3).
    #[validate(length(min = 1, max = 2000))]
    pub message_summary: String,

    /// Agents disponibles pour réception.
    #[validate(length(min = 1, max = 20))]
    pub available_agents: Vec<AgentRef>,

    /// Vecteurs de capacités par agent (clé = AgentRef).
    pub agent_capabilities: HashMap<AgentRef, CapabilityVector>,

    /// Budget contexte restant par agent (en tokens).
    pub context_budgets: HashMap<AgentRef, u32>,
}

/// Sortie du Filtreur.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterOutput {
    /// Décision par agent (ordre conservé).
    pub routing: Vec<(AgentRef, RoutingDecision)>,
}

/// Décision pour un agent (envoi / skip / envoi compressé).
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum RoutingDecision {
    /// Envoyer tel quel.
    Send {
        /// Priorité 0-10 (10 = urgent).
        priority: u8,
        /// Raison textuelle pour debug.
        reason: String,
    },
    /// Ne pas envoyer.
    Skip {
        /// Raison du skip.
        reason: String,
    },
    /// Envoyer une version compressée (ratio cible).
    SendCompressed {
        /// Ratio cible (ex: 3.0 = compresser 3x).
        ratio: f32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_decision_send_roundtrip() {
        let d = RoutingDecision::Send {
            priority: 7,
            reason: "high similarity".into(),
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: RoutingDecision = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn routing_decision_skip_roundtrip() {
        let d = RoutingDecision::Skip {
            reason: "below threshold".into(),
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: RoutingDecision = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn filter_input_validates() {
        let mut caps = HashMap::new();
        caps.insert("claude".to_string(), vec![0.1; 1024]);
        let mut budgets = HashMap::new();
        budgets.insert("claude".to_string(), 4000);
        let input = FilterInput {
            message_summary: "Fed maintained rates".into(),
            available_agents: vec!["claude".into()],
            agent_capabilities: caps,
            context_budgets: budgets,
        };
        assert!(input.validate().is_ok());
    }

    #[test]
    fn no_agents_fails_validation() {
        let input = FilterInput {
            message_summary: "x".into(),
            available_agents: vec![], // empty
            agent_capabilities: HashMap::new(),
            context_budgets: HashMap::new(),
        };
        assert!(input.validate().is_err());
    }
}
