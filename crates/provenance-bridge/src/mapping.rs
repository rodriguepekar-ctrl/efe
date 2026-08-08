// SPDX-License-Identifier: UNLICENSED
//! **Le mapping figé** — les conversions élémentaires pile A → pile B.
//!
//! Chaque fonction de ce module est **pure, déterministe et gelée**. Elles sont isolées ici, et
//! non dispersées dans [`crate::derive`], pour une raison précise : ce sont elles qui décident de
//! l'**identité** et du **contenu** des nœuds scellés. Les changer change les sceaux ; les
//! regarder toutes d'un coup doit donc être possible sur un seul écran.
//!
//! # Pourquoi une version de mapping
//!
//! Le champ `version` d'un [`Node::AgentAct`](provenance_dag::Node) est renseigné par
//! [`MAPPING_VERSION`] — la version **du pont**, pas celle d'un schéma emprunté ailleurs. C'est
//! délibéré : ce que le nœud doit dater, c'est la *règle de traduction* qui l'a produit. Si le
//! mapping évolue, les bundles nouvellement dérivés le disent d'eux-mêmes, et un auditeur peut
//! distinguer « le run a changé » de « la règle de lecture a changé ». Ne jamais renuméroter à la
//! baisse, ne jamais réutiliser une valeur.

use std::time::{SystemTime, UNIX_EPOCH};

use efe_swarm_provenance::AgentCallRecord;

/// La version de la **règle de traduction** (et non celle d'un schéma tiers). Voir l'en-tête.
pub const MAPPING_VERSION: &str = "fil-p/p1.v1";

/// Le préfixe imposé aux empreintes placées dans un champ que la pile B nomme `output`.
///
/// La pile B documente `AgentAct.output` comme « la sortie attestée — gardée pour l'audit »,
/// c'est-à-dire une valeur. La pile A, par minimisation, ne détient que son **empreinte**. Écrire
/// l'empreinte nue produirait un bundle qu'un lecteur croirait plus riche qu'il ne l'est ; le
/// préfixe rend la confusion **impossible à l'œil**, dans le nœud lui-même et donc sous le sceau.
pub const HASH_PREFIX: &str = "blake3:";

/// Ce qu'on écrit quand une empreinte attendue est **absente** de la source.
///
/// Volontairement **sans** [`HASH_PREFIX`] : un lecteur — humain ou programme — qui filtre sur le
/// préfixe ne peut pas prendre cette valeur pour une empreinte. Écrire une chaîne vide, ou pire un
/// hash de vide, laisserait croire à une empreinte réelle de contenu nul.
pub const HASH_ABSENT: &str = "absent";

/// La largeur du numéro de séquence dans un identifiant de nœud.
///
/// Six chiffres ordonnent lexicographiquement jusqu'à un million d'enregistrements par run, ce qui
/// est très au-delà de tout run observé. Au-delà, l'identifiant reste **unique** (le nombre déborde
/// simplement la largeur) : on perd le tri lexicographique, jamais la correction.
const SEQ_WIDTH: usize = 6;

/// L'identifiant d'un nœud d'acte : `{run}::{seq:06}-{agent}`.
///
/// Trois exigences se rencontrent ici, et aucune n'est négociable :
/// - **unique** dans le DAG — la pile B rejette un doublon (`DagError::DuplicateId`) ; l'unicité
///   vient de `seq`, strictement croissant dans une chaîne, et **non** d'`agent_id`, qu'un run
///   peut porter plusieurs fois ;
/// - **ordonnant** — le tri lexicographique suit l'ordre de la chaîne, ce qui rend un bundle
///   lisible sans outil ;
/// - **lisible** — `agent_id` reste visible pour un humain, et la convention `{cas}::{étape}` de
///   la pile B (`provenance-capture::tap`) est respectée : le run est le *cas*.
pub fn act_node_id(run_id: uuid::Uuid, r: &AgentCallRecord) -> String {
    format!("{run_id}::{seq:0width$}-{agent}", seq = r.seq, width = SEQ_WIDTH, agent = r.agent_id.as_str())
}

/// L'identifiant du nœud d'évidence attaché à un acte Mode C : l'identifiant de l'acte, suffixé.
///
/// Le suffixe garantit qu'évidence et acte ne peuvent pas collisionner, tout en gardant le lien
/// **lisible** : l'évidence d'un acte porte visiblement son nom.
pub fn evidence_node_id(act_id: &str) -> String {
    format!("{act_id}#llm")
}

/// L'identifiant d'une arête causale : `{parent}->{enfant}`.
///
/// Un couple (parent, enfant) est unique dans un DAG — deux arêtes identiques seraient la même
/// arête. L'identifiant est donc naturellement unique, et directement lisible.
pub fn edge_node_id(from_act_id: &str, to_act_id: &str) -> String {
    format!("{from_act_id}->{to_act_id}")
}

/// Le champ `model` d'un acte.
///
/// Un agent Mode A est du Rust déterministe : il n'a pas de modèle, et prétendre le contraire
/// serait une invention. On écrit donc sa **nature** (`mode_a`). Un agent Mode C porte, lui, une
/// identité de modèle réelle, que l'on rend sous la forme `mode_c:{provider}/{model_id}` — le
/// fournisseur ET le modèle, parce que « gpt-4o » chez deux fournisseurs n'est pas la même chose.
/// Sans identité disponible, on ne devine pas : on écrit `mode_c` nu.
pub fn model_of(r: &AgentCallRecord, model_identity: Option<(&str, &str)>) -> String {
    match (r.agent_kind, model_identity) {
        (efe_swarm_provenance::AgentKind::ModeC, Some((provider, model_id))) => {
            format!("mode_c:{provider}/{model_id}")
        }
        (kind, _) => kind.as_str().to_string(),
    }
}

/// Une empreinte, préfixée pour ce qu'elle est (voir [`HASH_PREFIX`]).
pub fn tagged_hash(hex: &str) -> String {
    format!("{HASH_PREFIX}{hex}")
}

/// `SystemTime` → millisecondes depuis l'époque.
///
/// La pile B date ses nœuds en `u64`. La milliseconde est le compromis retenu : elle sépare deux
/// enregistrements consécutifs d'un run réel sans porter une précision que la source ne garantit
/// pas. Une date **antérieure à l'époque** n'est pas convertie silencieusement en 0 — elle est une
/// erreur, remontée par l'appelant : une horloge qui ment est un fait à dire, pas à arrondir.
pub fn millis_since_epoch(t: SystemTime) -> Option<u64> {
    t.duration_since(UNIX_EPOCH).ok().map(|d| {
        // u128 -> u64 : sature plutôt que de tronquer par enroulement. La borne est à ~584 millions
        // d'années ; l'atteindre signalerait une horloge absurde, jamais un run réel.
        u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn hash_prefix_makes_a_fingerprint_unmistakable() {
        let h = tagged_hash("ab12");
        assert!(h.starts_with("blake3:"), "une empreinte ne peut pas passer pour une sortie");
        assert_eq!(h, "blake3:ab12");
    }

    #[test]
    fn evidence_and_edge_ids_cannot_collide_with_an_act() {
        let act = "run::000001-gen";
        assert_ne!(evidence_node_id(act), act);
        assert_ne!(edge_node_id(act, act), act);
        assert_ne!(evidence_node_id(act), edge_node_id(act, act));
    }

    #[test]
    fn millis_conversion_is_exact_and_rejects_pre_epoch() {
        assert_eq!(millis_since_epoch(UNIX_EPOCH), Some(0));
        assert_eq!(millis_since_epoch(UNIX_EPOCH + Duration::from_millis(1_234)), Some(1_234));
        // Une date antérieure à l'époque n'est PAS convertie : elle remonte.
        assert_eq!(millis_since_epoch(UNIX_EPOCH - Duration::from_millis(1)), None);
    }

    #[test]
    fn mapping_version_is_frozen() {
        // Ce test n'a l'air de rien : il rend TOUT changement de règle de traduction délibéré,
        // puisqu'il faut venir ici pour l'autoriser. C'est sa seule fonction, et elle suffit.
        assert_eq!(MAPPING_VERSION, "fil-p/p1.v1");
    }
}
