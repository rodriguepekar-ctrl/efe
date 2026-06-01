//! `RegistryEvent` — événements émis par le registre pour la télémétrie.
//!
//! Un consommateur (M11) peut souscrire via
//! [`crate::alignment::registry::InMemoryRegistry::take_events_receiver`]
//! et recevoir les événements au fil de l'eau. Best-effort — un
//! consommateur absent n'empêche pas le registre de fonctionner.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::alignment::breach::BreachReport;
use crate::alignment::key::ConeKey;

/// Événement émis par le registre à destination des consommateurs externes.
///
/// `Clone` pour permettre plusieurs consommateurs (fan-out côté
/// consommateur), `Serialize`/`Deserialize` pour export JSON (audit log).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RegistryEvent {
    /// Un cône a été enregistré pour la première fois.
    Registered {
        /// Clé résultante.
        key: ConeKey,
        /// Timestamp Unix nanos.
        at_nanos: u64,
    },
    /// Un cône a été évolué en une nouvelle version.
    Evolved {
        /// Clé du parent.
        from: ConeKey,
        /// Clé du nouveau cône.
        to: ConeKey,
        /// Type d'opération (`append_anchor`, `remove_anchor`, ...).
        /// Stocké en `String` (plutôt que `&'static str`) pour permettre
        /// un round-trip serde.
        op_kind: String,
    },
    /// Un cône a été supprimé.
    Deleted {
        /// Clé du cône supprimé.
        key: ConeKey,
    },
    /// Un breach a été rapporté.
    BreachReported {
        /// Rapport complet.
        report: BreachReport,
    },
    /// Une mutation a été rejetée par la policy.
    PolicyRejected {
        /// Clé tentée (peut être `None` si l'erreur précède la
        /// construction de la clé, typiquement lors d'un `evolve` dont
        /// la version suivante n'a pas encore été calculée).
        attempted_key: Option<ConeKey>,
        /// Raison textuelle — **le message complet de l'EfeError
        /// sous-jacente**, incluant le code d'invariant (`I1`..`I7`)
        /// quand applicable. Stocké en `String` pour permettre un
        /// round-trip serde ; la stabilité nominale est acquise car
        /// les messages sont produits par le [`super::PolicyEngine`]
        /// (finis, documentés).
        reason: String,
    },
}

impl fmt::Display for RegistryEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registered { key, at_nanos } => {
                write!(f, "registered[{key}]@{at_nanos}")
            }
            Self::Evolved { from, to, op_kind } => {
                write!(f, "evolved[{from}->{to}] op={op_kind}")
            }
            Self::Deleted { key } => write!(f, "deleted[{key}]"),
            Self::BreachReported { report } => write!(f, "{report}"),
            Self::PolicyRejected {
                attempted_key,
                reason,
            } => match attempted_key {
                Some(k) => write!(f, "rejected[{k}] {reason}"),
                None => write!(f, "rejected[-] {reason}"),
            },
        }
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alignment::breach::BreachContext;
    use crate::types::SwarmId;

    fn key() -> ConeKey {
        ConeKey::new("acme", "t", 1).unwrap()
    }

    #[test]
    fn display_registered() {
        let e = RegistryEvent::Registered {
            key: key(),
            at_nanos: 42,
        };
        assert!(e.to_string().contains("registered"));
        assert!(e.to_string().contains("acme/t@v1"));
    }

    #[test]
    fn display_evolved() {
        let e = RegistryEvent::Evolved {
            from: key(),
            to: ConeKey::new("acme", "t", 2).unwrap(),
            op_kind: "append_anchor".to_string(),
        };
        let s = e.to_string();
        assert!(s.contains("append_anchor"));
        assert!(s.contains("->"));
    }

    #[test]
    fn display_deleted() {
        let e = RegistryEvent::Deleted { key: key() };
        assert!(e.to_string().contains("deleted"));
    }

    #[test]
    fn display_policy_rejected_some() {
        let e = RegistryEvent::PolicyRejected {
            attempted_key: Some(key()),
            reason: "I1: K exceeds max_anchors".to_string(),
        };
        let s = e.to_string();
        assert!(s.contains("rejected"));
        assert!(s.contains("I1"));
    }

    #[test]
    fn display_policy_rejected_none() {
        let e = RegistryEvent::PolicyRejected {
            attempted_key: None,
            reason: "invalid key".to_string(),
        };
        let s = e.to_string();
        assert!(s.contains("rejected[-]"));
    }

    #[test]
    fn display_breach() {
        let report = BreachReport {
            cone_key: key(),
            swarm_id: SwarmId::new(),
            observed_delta: 0.9,
            cone_radius: 0.5,
            timestamp_nanos: 100,
            context: BreachContext::Dissolution,
        };
        let e = RegistryEvent::BreachReported { report };
        assert!(e.to_string().contains("breach"));
    }

    #[test]
    fn serde_roundtrip_registered() {
        let e = RegistryEvent::Registered {
            key: key(),
            at_nanos: 1234,
        };
        let s = serde_json::to_string(&e).unwrap();
        let back: RegistryEvent = serde_json::from_str(&s).unwrap();
        match back {
            RegistryEvent::Registered { key: k, at_nanos } => {
                assert_eq!(k, key());
                assert_eq!(at_nanos, 1234);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn serde_roundtrip_deleted() {
        let e = RegistryEvent::Deleted { key: key() };
        let s = serde_json::to_string(&e).unwrap();
        let back: RegistryEvent = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, RegistryEvent::Deleted { .. }));
    }
}
