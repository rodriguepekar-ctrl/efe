// SPDX-License-Identifier: UNLICENSED
//! # `conservation` — M6, versant S1 : ce qu'on chiffre à la capture, et ce qui survit au broyage.
//!
//! [`provenance_dag::shred`] fournit la mécanique — un chiffre par cas, dont détruire la clé rend le
//! contenu irrécupérable **sans qu'un octet du DAG ne bouge**. Ce module décide *à quoi* elle
//! s'applique : quels champs d'un nœud sont enrobés au moment de la capture, et — beaucoup plus
//! important — **lesquels ne le sont pas, et pourquoi**.
//!
//! ## L'inventaire : ce qu'un cas broyé laisse derrière lui
//!
//! Un broyage ne fait pas disparaître le nœud. Il faut donc pouvoir dire, exactement, ce qu'un
//! auditeur voit encore d'un cas dont le contenu est détruit. Cette table **est** cette réponse.
//!
//! | champ | après broyage | pourquoi |
//! |---|---|---|
//! | `AgentAct.output` | **détruit** | la décision, en clair — le contenu même |
//! | `Human.justification` | **détruit** | texte libre écrit par une personne |
//! | `Human.human_id` | **détruit** | il **désigne** une personne, pas seulement son acte |
//! | `Evidence.retrieved_ref` | **détruit** | un chemin de document désigne souvent son sujet |
//! | `*_hash` (`inputs`, `artifact`, `query`, `content`) | survit | déjà minimisé : une empreinte, jamais le contenu |
//! | `node_id` / `edge_id` / `evidence_id` | survit | le squelette : les chiffrer casserait les références **et** le nom du cas à broyer |
//! | `timestamp` | survit | c'est lui qui décide de l'expiration |
//! | `agent_role`, `model`, `version`, `tool`, `action` | survit | métadonnée d'exploitation, pas du contenu |
//! | `from_node`, `to_node`, `target_node`, `consumed_by_node`, `evidence_refs` | survit | des références ; les chiffrer romprait l'intégrité structurelle |
//! | `artifact_type` | survit | un nom de type, pas une valeur |
//!
//! Ce qui reste dit donc : *un acte de ce rôle a eu lieu à cette date, dans ce cas, relié à
//! ceux-là*. Ce qui part : *ce qu'il disait*. C'est précisément la limitation de conservation — et
//! **pas** l'effacement d'une trace, que l'append-only refuse par construction et qu'il serait
//! malhonnête de promettre.
//!
//! ## L'identifiant de cas survit, et c'est un choix
//!
//! `node_id` porte `{cas}::{étape}`. Il n'est pas chiffré, pour deux raisons qui se renforcent :
//! sans lui on ne saurait plus **quel** cas broyer, et les références (`evidence_refs`,
//! `target_node`…) doivent rester résolubles ou `reprove_dag` refuse tout le DAG. Conséquence à
//! assumer : **si l'identifiant de cas est lui-même une donnée personnelle** — un numéro de dossier
//! nominatif, par exemple — le broyage ne l'atteint pas. C'est un défaut de conception en amont,
//! pas un défaut de ce module ; il faut le dire plutôt que de laisser croire le contraire.

//! ## Le vocabulaire, et ce que ce module N'EST PAS
//!
//! La feuille de route détecte ce module par `grep -rniE 'retention|purge|ttl'`. Ces trois mots
//! méritent d'être tranchés ici, parce que deux seulement s'appliquent — et que le troisième
//! désigne précisément ce qu'on ne peut pas faire.
//!
//! * **`retention`** — oui. `Conservation::horizon_s` EST une politique de rétention : au-delà de
//!   cet âge, un cas ne se lit plus. C'est la limitation de conservation du GDPR art. 5(1)(e).
//! * **`ttl`** — oui, par cas. L'horizon se compte depuis l'**ouverture** du cas (`depuis`), pas
//!   depuis son dernier accès : une durée de vie, pas une expiration de cache.
//! * **`purge`** — **NON**, et jamais. Une purge supprime des enregistrements. Ici, `chaîne ←
//!   HMAC(clé, chaîne ‖ octets_canoniques(nœud))` : retirer un record casse son sceau, tous les
//!   suivants, et les références qui pointaient vers lui. **Rien n'est jamais supprimé.** Ce qui
//!   disparaît est la capacité de LIRE, par destruction de la clé — le nœud, son rôle, sa date et
//!   ses arêtes demeurent. Employer le mot « purge » ferait croire à un effacement de trace, et un
//!   auditeur qui l'entendrait chercherait des records absents qui sont là.
//!
//! (Cette section existe parce que le détecteur de la feuille de route renvoyait **zéro** sur ce
//! module : il est écrit en français — « conservation », « broyage », « horizon » — et sa
//! terminologie ne croisait aucun des trois termes. M2 avait rencontré exactement cela et l'avait
//! corrigé de la même façon. Un module qu'aucune preuve ne détecte se fait oublier.)
//!

use std::collections::BTreeMap;

use provenance_dag::shred::{sceller, CaseKey, EtatCas, Lecture, Trousseau};
use provenance_dag::Node;

/// Le cas auquel un identifiant appartient : tout ce qui précède le premier `::`.
///
/// C'est la convention de [`crate::node_id`]. Un identifiant qui ne la suit pas — ceux d'un run
/// d'essaim, par exemple — est son propre cas : mieux vaut un cas dégénéré qu'une confusion de
/// périmètre.
#[must_use]
pub fn cas_de(id: &str) -> &str {
    match id.find("::") {
        Some(i) => &id[..i],
        None => id,
    }
}

/// Le contexte de chiffrement d'un champ : `{identifiant}:{champ}`.
///
/// Unique par champ, parce que les identifiants de nœud le sont (le DAG refuse les doublons à
/// l'append). C'est la seule condition de sûreté du chiffre à flot.
fn contexte(id: &str, champ: &str) -> String {
    format!("{id}:{champ}")
}

/// Enrobe les champs effaçables d'un nœud avec la clé de son cas.
///
/// À appeler **avant** `SealedDag::append` : c'est le chiffré qui doit être scellé, sinon le clair
/// resterait dans la chaîne et le broyage ne détruirait rien de ce qui compte.
///
/// Enrober deux fois est sans danger mais absurde (on chiffrerait un chiffré) ; le nœud produit ici
/// n'est pas destiné à repasser par cette fonction.
#[must_use]
pub fn enrober(noeud: Node, cle: &CaseKey) -> Node {
    enrober_avec(noeud, |ctx, clair| sceller(cle, ctx, clair))
}

/// Enrobe **par le trousseau**, sans jamais toucher la clé — la porte du chemin servi.
///
/// `Err(SansCle)` quand le cas n'a pas de clé vivante. L'appelant doit alors décider, et le déclarer :
/// sceller en clair (le nœud existe, mais ne sera jamais effaçable) ou refuser (le nœud est perdu).
/// Ce qu'il ne doit pas faire, c'est choisir en silence — c'est pour cela que ce n'est pas un
/// `Option<Node>` qui se `unwrap_or(noeud)` sans qu'on y pense.
pub fn enrober_par(noeud: Node, trousseau: &Trousseau) -> Result<Node, SansCle> {
    let cas = cas_de(noeud.id()).to_string();
    match trousseau.etat(&cas) {
        EtatCas::Vivant { .. } => Ok(enrober_avec(noeud, |ctx, clair| {
            trousseau.sceller_pour(&cas, ctx, clair).expect("cas vivant ⟹ clé présente")
        })),
        EtatCas::Broye { le } => Err(SansCle::Broye { cas, le }),
        EtatCas::Inconnu => Err(SansCle::Inconnu { cas }),
    }
}

/// Pourquoi un nœud n'a pas pu être enrobé. **Deux** raisons, jamais confondues : l'une est un fait
/// accompli (ce cas a été détruit — ingérer dedans est une erreur de séquence), l'autre une absence
/// (personne n'a ouvert ce cas — c'est une omission de câblage).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SansCle {
    /// Le cas a été broyé à cette date : y ajouter un nœud produirait un chiffré illisible à jamais.
    Broye { cas: String, le: u64 },
    /// Aucune clé n'a été confiée pour ce cas.
    Inconnu { cas: String },
}

impl SansCle {
    /// Le cas concerné — ce qu'on inscrit dans un compteur ou un journal.
    #[must_use]
    pub fn cas(&self) -> &str {
        match self {
            SansCle::Broye { cas, .. } | SansCle::Inconnu { cas } => cas,
        }
    }
}

/// Ce nœud porte-t-il quelque chose d'effaçable ?
///
/// Sert au point de scellement à ne compter dans « scellé en clair faute de clé » que les nœuds qui
/// **perdent** vraiment quelque chose. Une arête n'a aucun contenu : la compter ferait gonfler un
/// compteur de données ineffaçables avec des nœuds qui n'ont rien à effacer, et enverrait un
/// exploitant chercher un problème qui n'existe pas. Un compteur qui crie pour rien use la confiance
/// qu'on lui accorde.
///
/// Exhaustif sans joker, pour la même raison que [`enrober_avec`] : une variante future qui
/// porterait du contenu doit casser la compilation, pas passer pour vide.
#[must_use]
pub fn a_du_contenu_effacable(noeud: &Node) -> bool {
    match noeud {
        Node::AgentAct { .. } | Node::Human { .. } | Node::Evidence { .. } => true,
        Node::Edge { .. } => false,
    }
}

/// L'unique implémentation : `f` reçoit `(contexte, clair)` et rend l'enveloppe.
///
/// Le `match` vit ici et **nulle part ailleurs** : ajouter une variante de `Node` qui porte du
/// contenu et oublier de l'enrober serait le défaut le plus coûteux de ce module (un clair scellé
/// pour toujours, sans que rien ne proteste). Un seul endroit à relire.
fn enrober_avec(noeud: Node, f: impl Fn(&str, &str) -> String) -> Node {
    match noeud {
        Node::AgentAct {
            node_id,
            agent_role,
            inputs_hash,
            model,
            version,
            output,
            evidence_refs,
            timestamp,
        } => {
            let output = f(&contexte(&node_id, "output"), &output);
            Node::AgentAct {
                node_id,
                agent_role,
                inputs_hash,
                model,
                version,
                output,
                evidence_refs,
                timestamp,
            }
        }
        Node::Human { node_id, human_id, action, target_node, justification, timestamp } => {
            let human_id = f(&contexte(&node_id, "human_id"), &human_id);
            let justification = f(&contexte(&node_id, "justification"), &justification);
            Node::Human { node_id, human_id, action, target_node, justification, timestamp }
        }
        Node::Evidence {
            evidence_id,
            tool,
            query_hash,
            retrieved_ref,
            content_hash,
            consumed_by_node,
            timestamp,
        } => {
            let retrieved_ref = f(&contexte(&evidence_id, "retrieved_ref"), &retrieved_ref);
            Node::Evidence {
                evidence_id,
                tool,
                query_hash,
                retrieved_ref,
                content_hash,
                consumed_by_node,
                timestamp,
            }
        }
        // Une ARÊTE ne porte aucun contenu : un identifiant, deux références, une empreinte, un nom
        // de type. Rien à effacer — et le dire explicitement vaut mieux qu'un `_ => noeud` qui
        // avalerait en silence une variante future portant, elle, du contenu.
        Node::Edge { .. } => noeud,
    }
}

/// Relit les champs effaçables d'un nœud à travers un trousseau.
///
/// Rend, par nom de champ, une [`Lecture`] qui **dit pourquoi** quand elle ne rend pas de contenu :
/// déchiffré, broyé à telle date, clé absente, jamais chiffré. Un `Option::None` aurait confondu ces
/// quatre situations, dont deux au moins appellent des réponses opposées.
#[must_use]
pub fn lire(noeud: &Node, trousseau: &Trousseau) -> BTreeMap<&'static str, Lecture> {
    let mut out = BTreeMap::new();
    match noeud {
        Node::AgentAct { node_id, output, .. } => {
            let cas = cas_de(node_id);
            out.insert("output", trousseau.lire(cas, &contexte(node_id, "output"), output));
        }
        Node::Human { node_id, human_id, justification, .. } => {
            let cas = cas_de(node_id);
            out.insert("human_id", trousseau.lire(cas, &contexte(node_id, "human_id"), human_id));
            out.insert(
                "justification",
                trousseau.lire(cas, &contexte(node_id, "justification"), justification),
            );
        }
        Node::Evidence { evidence_id, retrieved_ref, .. } => {
            let cas = cas_de(evidence_id);
            out.insert(
                "retrieved_ref",
                trousseau.lire(cas, &contexte(evidence_id, "retrieved_ref"), retrieved_ref),
            );
        }
        Node::Edge { .. } => {}
    }
    out
}

/// La politique de conservation : un horizon, en secondes.
///
/// `reg-evidence` le dit de l'art. 12 de l'AI Act — *« la politique de rétention/conservation reste
/// une décision du déployeur »*. Ce type ne choisit donc **aucune** durée par défaut : il exécute
/// celle qu'on lui donne. Un défaut implicite serait une décision réglementaire prise à la place du
/// client, et prise en silence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conservation {
    /// Au-delà de cet âge, un cas est expiré. `0` broierait tout immédiatement — accepté, parce
    /// qu'un refus déguiserait une politique en erreur de configuration.
    pub horizon_s: u64,
}

impl Conservation {
    #[must_use]
    pub fn nouvelle(horizon_s: u64) -> Self {
        Self { horizon_s }
    }

    /// Broie ce qui a expiré et rend le compte rendu — jamais un simple nombre.
    pub fn appliquer(&self, trousseau: &mut Trousseau, maintenant: u64) -> Balayage {
        let broyes = trousseau.broyer_expires(self.horizon_s, maintenant);
        let (vivants, total_broyes) = trousseau.compte();
        Balayage { broyes, vivants, total_broyes, horizon_s: self.horizon_s, a: maintenant }
    }
}

/// Ce qu'un passage de la politique a fait — avec son **dénominateur**.
///
/// « 3 cas broyés » ne se lit pas : trois sur quatre est un balayage, trois sur trois mille en est
/// un autre. La leçon du compteur sans dénominateur, déjà payée une fois du côté des dossiers
/// réglementaires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Balayage {
    /// Les cas broyés à ce passage, dans l'ordre.
    pub broyes: Vec<String>,
    /// Les cas encore vivants après le passage.
    pub vivants: usize,
    /// Les cas broyés depuis toujours — dont ceux des passages précédents.
    pub total_broyes: usize,
    /// L'horizon appliqué, retenu avec le résultat : sans lui, le compte n'est pas interprétable.
    pub horizon_s: u64,
    /// L'instant du passage.
    pub a: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tap::node_id;
    use provenance_dag::shred::EtatCas;

    fn cle(n: u8) -> CaseKey {
        CaseKey::from_bytes([n; 32])
    }

    fn acte(cas: &str, etape: &str, sortie: &str, t: u64) -> Node {
        Node::AgentAct {
            node_id: node_id(cas, etape),
            agent_role: "verdict".into(),
            inputs_hash: "h".into(),
            model: "phi4".into(),
            version: "1".into(),
            output: sortie.into(),
            evidence_refs: vec![],
            timestamp: t,
        }
    }

    #[test]
    fn le_cas_se_lit_dans_lidentifiant() {
        assert_eq!(cas_de("SUP-2024::sanction"), "SUP-2024");
        assert_eq!(cas_de("SUP-2024::a::b"), "SUP-2024", "le PREMIER `::` sépare");
        assert_eq!(cas_de("run-sans-convention"), "run-sans-convention", "cas dégénéré, pas confusion");
    }

    #[test]
    fn enrober_chiffre_la_sortie_et_rien_dautre() {
        let n = enrober(acte("C1", "x", "COMPLIANT", 10), &cle(1));
        let Node::AgentAct { node_id, output, agent_role, model, timestamp, inputs_hash, .. } = &n
        else {
            panic!("variante inattendue")
        };
        assert!(provenance_dag::est_enveloppe(output), "la sortie est enrobée");
        assert_eq!(node_id, "C1::x", "l'identifiant survit — sinon on ne saurait plus quoi broyer");
        assert_eq!((agent_role.as_str(), model.as_str(), *timestamp), ("verdict", "phi4", 10));
        assert_eq!(inputs_hash, "h", "une empreinte est déjà minimisée");
    }

    #[test]
    fn une_arete_na_rien_a_effacer() {
        let e = Node::Edge {
            edge_id: "C1::e".into(),
            from_node: "C1::a".into(),
            to_node: "C1::b".into(),
            artifact_hash: "h".into(),
            artifact_type: "json".into(),
            timestamp: 1,
        };
        assert_eq!(enrober(e.clone(), &cle(1)), e, "inchangée");
        assert!(lire(&e, &Trousseau::new()).is_empty(), "aucun champ effaçable");
    }

    #[test]
    fn les_deux_champs_dun_humain_sont_enrobes() {
        let h = Node::Human {
            node_id: node_id("C1", "revue"),
            human_id: "alice@exemple".into(),
            action: provenance_dag::HumanAction::Approve,
            target_node: node_id("C1", "x"),
            justification: "revue faite le 12, pièces au dossier".into(),
            timestamp: 20,
        };
        let e = enrober(h, &cle(2));
        let Node::Human { human_id, justification, target_node, .. } = &e else { panic!() };
        assert!(provenance_dag::est_enveloppe(human_id), "il DÉSIGNE une personne");
        assert!(provenance_dag::est_enveloppe(justification));
        assert_eq!(target_node, "C1::x", "une référence reste résoluble");

        let mut t = Trousseau::new();
        t.poser("C1", cle(2), 0);
        let vue = lire(&e, &t);
        assert_eq!(vue["human_id"], Lecture::Ouvert("alice@exemple".into()));
        assert_eq!(vue["justification"], Lecture::Ouvert("revue faite le 12, pièces au dossier".into()));
    }

    #[test]
    fn un_champ_jamais_chiffre_se_lit_toujours() {
        // Le passé : un nœud scellé avant l'activation d'un trousseau porte son contenu en clair,
        // et rien ne peut le reprendre. La lecture doit le DIRE, pas échouer.
        let n = acte("C1", "x", "PASS", 1);
        let mut t = Trousseau::new();
        t.poser("C1", cle(1), 0);
        assert_eq!(lire(&n, &t)["output"], Lecture::EnClair("PASS".into()));
    }

    #[test]
    fn la_politique_rend_un_compte_rendu_avec_son_denominateur() {
        let mut t = Trousseau::new();
        t.poser("vieux-1", cle(1), 0);
        t.poser("vieux-2", cle(2), 100);
        t.poser("recent", cle(3), 900);

        let b = Conservation::nouvelle(500).appliquer(&mut t, 1000);
        assert_eq!(b.broyes, vec!["vieux-1".to_string(), "vieux-2".to_string()]);
        assert_eq!((b.vivants, b.total_broyes), (1, 2), "2 sur 3 — le compte seul ne se lirait pas");
        assert_eq!((b.horizon_s, b.a), (500, 1000), "l'horizon voyage avec son résultat");
        assert!(matches!(t.etat("recent"), EtatCas::Vivant { .. }));
    }

    #[test]
    fn un_second_passage_ne_rebroie_rien() {
        let mut t = Trousseau::new();
        t.poser("c", cle(1), 0);
        let p = Conservation::nouvelle(10);
        assert_eq!(p.appliquer(&mut t, 100).broyes.len(), 1);
        let b2 = p.appliquer(&mut t, 200);
        assert!(b2.broyes.is_empty(), "idempotent : la clé n'existe plus");
        assert_eq!(b2.total_broyes, 1, "mais le fait reste compté");
    }
}
