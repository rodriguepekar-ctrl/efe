// SPDX-License-Identifier: UNLICENSED
//! # `family` — la clé qui regroupe les tâches « semblables »
//!
//! Pour que la mémoire **généralise** (qu'un run profite des runs passés sur des tâches du
//! même genre), il faut regrouper les tâches sous une **clé de famille** stable. MEM-1 fournit
//! les deux empreintes **pures et déterministes** du spectre de l'architecture (§3.2) :
//!
//! - **Exacte** — hash de la tâche *normalisée* (valeurs comprises) + topologie + agents.
//!   Ne regroupe que les tâches **identiques** ; le moat reste trivialement propre.
//! - **Structurelle** (le défaut souverain) — hash de la **FORME** de la tâche (les clés et
//!   les *types*, jamais les valeurs) + topologie + agents. Regroupe les tâches de **même
//!   forme** (« toute classification de texte en une catégorie » = une famille) tout en
//!   restant parfaitement déterministe.
//!
//! L'empreinte **sémantique** (embedding local, Mode C) est un opt-in de MEM-3 : sa
//! non-déterminisme est absorbée par N17 (l'embedding est un appel LLM enregistré, rejoué —
//! pas recalculé), exactement comme tout Mode C. Elle n'a pas sa place dans ce crate pur.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{content_hash_of, hex32};

/// La clé d'une famille de tâches — un hash hex stable. `Ord`/`Hash` pour l'indexation du
/// store et l'ordre canonique. Opaque : sa valeur n'a de sens que par comparaison.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FamilyKey(String);

impl FamilyKey {
    /// Construit une clé à partir d'un hash déjà calculé (usage interne / tests).
    pub fn from_hash(h: &[u8; 32]) -> Self {
        FamilyKey(hex32(h))
    }

    /// **ADR-019 (MEM-3)** : reconstruit une clé depuis sa forme hex (ex. un override de
    /// famille fourni par le DTO de la passerelle, ou la famille consignée dans un run). La
    /// chaîne EST l'identité opaque — pas de re-validation (une clé inconnue donne simplement
    /// une famille vide à la lecture).
    pub fn from_hex(s: impl Into<String>) -> Self {
        FamilyKey(s.into())
    }
    /// La représentation hex (persistance / fil / indexation).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for FamilyKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Le mode d'empreinte (le caller choisit ; le défaut souverain est `Structural`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FamilyMode {
    /// Hash de la tâche normalisée (valeurs comprises) — ne regroupe que l'identique.
    Exact,
    /// Hash de la FORME (clés + types, valeurs ignorées) — regroupe par structure. **Défaut.**
    #[default]
    Structural,
}

/// **L'empreinte de famille — pure et déterministe.** Combine la tâche (exacte ou réduite à sa
/// forme), le nom de topologie et la liste d'agents (**triée** — l'ordre de déclaration ne doit
/// pas créer deux familles). Hash canonique (clés triées) → insensible à l'ordre de
/// sérialisation. Deux appels avec les mêmes entrées donnent **toujours** la même clé.
pub fn family_key(task: &Value, topology: &str, agents: &[String], mode: FamilyMode) -> FamilyKey {
    let task_part = match mode {
        FamilyMode::Exact => task.clone(),
        FamilyMode::Structural => shape(task),
    };
    // Agents triés + dédupliqués : la famille ne dépend pas de l'ordre de déclaration.
    let mut sorted_agents: Vec<&String> = agents.iter().collect();
    sorted_agents.sort();
    sorted_agents.dedup();
    let envelope = json!({
        "mode": mode,
        "topology": topology,
        "agents": sorted_agents,
        "task": task_part,
    });
    FamilyKey::from_hash(&content_hash_of(&envelope))
}

/// La **FORME** d'une valeur JSON : on remplace chaque scalaire par une étiquette de **type**,
/// et l'on garde la structure (clés d'objet triées par `canonical_bytes` à la sérialisation ;
/// les tableaux sont réduits à la forme de l'**union** de leurs éléments, pour que la longueur
/// n'éclate pas les familles). Récursif, total, pur.
fn shape(v: &Value) -> Value {
    match v {
        Value::Null => json!("null"),
        Value::Bool(_) => json!("bool"),
        Value::Number(_) => json!("number"),
        Value::String(_) => json!("string"),
        Value::Object(map) => {
            // Map clé → forme(valeur). `canonical_bytes` triera les clés au hachage : on n'a
            // pas à trier ici, mais on le fait pour une forme lisible et stable.
            let mut obj = serde_json::Map::new();
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for k in keys {
                obj.insert(k.clone(), shape(&map[k]));
            }
            Value::Object(obj)
        }
        Value::Array(items) => {
            // Forme de l'union des éléments : on collecte les formes distinctes, triées —
            // un tableau de 3 strings et un tableau de 10 strings partagent la même forme.
            let mut shapes: Vec<Value> = items.iter().map(shape).collect();
            shapes.sort_by_key(|a| a.to_string());
            shapes.dedup();
            json!({ "$array_of": shapes })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_groups_same_shape_different_values() {
        // I-MEM (généralisation) : deux tâches de MÊME forme, valeurs différentes → MÊME famille.
        let a = json!({ "text": "bonjour le monde", "expected": "salutation" });
        let b = json!({ "text": "tout autre contenu", "expected": "x" });
        let fa = family_key(&a, "consensus", &["g".into(), "j".into()], FamilyMode::Structural);
        let fb = family_key(&b, "consensus", &["g".into(), "j".into()], FamilyMode::Structural);
        assert_eq!(fa, fb, "même forme (text:string, expected:string) → même famille");
    }

    #[test]
    fn structural_separates_different_shapes() {
        let a = json!({ "text": "x" });
        let b = json!({ "text": "x", "lang": "fr" }); // clé en plus → autre forme
        let c = json!({ "text": 42 }); // type différent → autre forme
        let fa = family_key(&a, "consensus", &[], FamilyMode::Structural);
        let fb = family_key(&b, "consensus", &[], FamilyMode::Structural);
        let fc = family_key(&c, "consensus", &[], FamilyMode::Structural);
        assert_ne!(fa, fb, "clé supplémentaire → famille distincte");
        assert_ne!(fa, fc, "type de valeur distinct → famille distincte");
    }

    #[test]
    fn exact_separates_different_values() {
        let a = json!({ "text": "alpha" });
        let b = json!({ "text": "beta" });
        assert_ne!(
            family_key(&a, "consensus", &[], FamilyMode::Exact),
            family_key(&b, "consensus", &[], FamilyMode::Exact),
            "exact : valeurs différentes → familles différentes"
        );
        assert_eq!(
            family_key(&a, "consensus", &[], FamilyMode::Exact),
            family_key(&a, "consensus", &[], FamilyMode::Exact),
            "exact : déterministe (même entrée → même clé)"
        );
    }

    #[test]
    fn topology_and_agents_are_part_of_the_family() {
        let t = json!({ "text": "x" });
        let f_cons = family_key(&t, "consensus", &["g".into()], FamilyMode::Structural);
        let f_gv = family_key(&t, "generate_verify", &["g".into()], FamilyMode::Structural);
        let f_two = family_key(&t, "consensus", &["g".into(), "j".into()], FamilyMode::Structural);
        assert_ne!(f_cons, f_gv, "topologie distincte → famille distincte");
        assert_ne!(f_cons, f_two, "set d'agents distinct → famille distincte");
    }

    #[test]
    fn agent_order_is_neutral() {
        // L'ordre de DÉCLARATION des agents ne crée pas deux familles (tri + dédup).
        let t = json!({ "text": "x" });
        let f1 = family_key(&t, "consensus", &["g".into(), "j1".into(), "j2".into()], FamilyMode::Structural);
        let f2 = family_key(&t, "consensus", &["j2".into(), "g".into(), "j1".into()], FamilyMode::Structural);
        assert_eq!(f1, f2, "ordre des agents neutre");
    }

    #[test]
    fn array_length_does_not_explode_families() {
        // I-MEM (généralisation) : un tableau de 3 et un tableau de 10 du même type → même forme.
        let a = json!({ "items": ["a", "b", "c"] });
        let b = json!({ "items": ["x", "y", "z", "w", "v", "u", "t", "s", "r", "q"] });
        assert_eq!(
            family_key(&a, "pipeline", &[], FamilyMode::Structural),
            family_key(&b, "pipeline", &[], FamilyMode::Structural),
            "la longueur du tableau n'éclate pas la famille (forme de l'union)"
        );
    }
}
