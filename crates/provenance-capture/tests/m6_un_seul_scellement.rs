// SPDX-License-Identifier: UNLICENSED
//! # Le garde de M6 — ce qui empêche la propriété de se perdre au module suivant.
//!
//! M6 tient sur deux faits fragiles, tous deux du genre qui se cassent en silence :
//!
//! 1. **Tout nœud passe par un seul point de scellement.** Les quatre `record_*` étaient quatre
//!    `self.dag.append(…)` identiques ; c'est précisément la forme où le cinquième diverge. Un tap
//!    ajouté plus tard qui scellerait directement produirait du clair, définitif, sans que rien ne
//!    proteste — et tout ce fichier de M6 continuerait de passer au vert.
//!
//! 2. **Aucun joker dans les `match` sur `Node`.** L'exhaustivité du compilateur est la seule chose
//!    qui garantit qu'une variante future portant du contenu sera *vue*. Un `_ => noeud` la rendrait
//!    invisible : le code compilerait, le contenu serait scellé en clair pour toujours.
//!
//! Aucun des deux ne se rattrape après coup : un clair scellé l'est pour la vie de la chaîne.
//! D'où un garde, et non une vigilance.

use std::collections::BTreeMap;

use provenance_capture::conservation::{enrober, lire, Conservation};
use provenance_capture::{CaptureStore, LlmCall};
use provenance_dag::shred::{CaseKey, Lecture, Trousseau};
use provenance_dag::{est_enveloppe, reprove_dag, Node, TenantKey};

const STORE: &str = include_str!("../src/store.rs");
const CONSERVATION: &str = include_str!("../src/conservation.rs");

/// Les scellements directs tolérés, **nommés**, avec leur raison. Doit rester à UN : celui du point
/// de décision unique lui-même.
const SCELLEMENTS_DIRECTS_ATTENDUS: usize = 1;

#[test]
fn un_seul_point_de_scellement_dans_le_store() {
    let n = STORE.matches("self.dag.append(").count();
    assert_eq!(
        n, SCELLEMENTS_DIRECTS_ATTENDUS,
        "il doit exister UN seul `self.dag.append(` — celui de `CaptureStore::sceller`. En trouver \
         {n} veut dire qu'un tap scelle sans passer par la conservation : son contenu sera en clair \
         dans la chaîne, définitivement, et aucun test de M6 ne le verra"
    );
}

#[test]
fn aucun_joker_ne_masque_une_variante_de_noeud() {
    // Le compilateur est le seul garde qui attrape une variante AJOUTÉE. On lui interdit de se taire.
    for (quoi, portion) in [
        ("enrober_avec", extraire(CONSERVATION, "fn enrober_avec")),
        ("lire", extraire(CONSERVATION, "pub fn lire")),
    ] {
        assert!(
            !portion.contains("_ =>"),
            "`{quoi}` ne doit contenir aucun joker : une variante de `Node` ajoutée demain doit \
             CASSER LA COMPILATION, pas être avalée en silence — le silence, ici, scelle du clair"
        );
        assert!(
            portion.contains("Node::Edge"),
            "`{quoi}` doit nommer chaque variante, y compris celle qui n'a rien à effacer"
        );
    }
}

/// Le chemin **servi** : un tap ordinaire, inchangé, produit désormais du chiffré — parce que le
/// point de scellement s'en charge, et non parce que l'appelant y a pensé.
#[test]
fn le_chemin_servi_enrobe_sans_que_le_tap_y_pense() {
    let mut store = CaptureStore::new(TenantKey::derive(b"racine", "acme"));
    store.avec_conservation(Trousseau::new());
    assert!(store.ouvrir_cas("D-1", CaseKey::from_bytes([5u8; 32]), 0));

    store.record_llm(&appel("D-1", "COMPLIANT", 0)).expect("ingestion");

    let Node::AgentAct { output, node_id, .. } = &store.records()[0].node else { panic!() };
    assert!(est_enveloppe(output), "le tap n'a pas changé, et pourtant la sortie est enrobée");
    assert_eq!(node_id, "D-1::verdict");
    assert!(store.sans_cle().is_empty(), "le cas avait sa clé");

    // Et le critère, à travers l'API servie.
    let racine = store.reprove().manifest_root.clone().expect("racine");
    let lu = lire(&store.records()[0].node, store.conservation().unwrap());
    assert_eq!(lu["output"], Lecture::Ouvert("COMPLIANT".into()));

    let b = Conservation::nouvelle(10).appliquer(store.conservation_mut().unwrap(), 100);
    assert_eq!(b.broyes, vec!["D-1".to_string()]);

    assert_eq!(
        store.reprove().manifest_root.as_ref(),
        Some(&racine),
        "après broyage, le store se re-prouve sur la MÊME racine"
    );
    let lu = lire(&store.records()[0].node, store.conservation().unwrap());
    assert_eq!(lu["output"], Lecture::Broye { le: 100 }, "et le contenu est parti");
}

/// Un cas jamais ouvert est scellé **en clair** — et compté par cas. Le compte est le seul endroit
/// où cette perte irréversible est visible ; un compteur nu se lirait « rien à signaler ».
#[test]
fn un_cas_sans_cle_est_scelle_en_clair_et_annonce() {
    let mut store = CaptureStore::new(TenantKey::derive(b"racine", "acme"));
    store.avec_conservation(Trousseau::new());
    store.record_llm(&appel("OUBLIE", "PASS", 0)).expect("ingestion");
    store.record_llm(&appel2("OUBLIE", "verdict-2", "PASS", 1)).expect("ingestion");

    let Node::AgentAct { output, .. } = &store.records()[0].node else { panic!() };
    assert_eq!(output, "PASS", "scellé en clair : le nœud d'audit existe, mais il est ineffaçable");
    assert_eq!(
        store.sans_cle(),
        &BTreeMap::from([("OUBLIE".to_string(), 2u64)]),
        "annoncé PAR CAS et compté : deux nœuds de « OUBLIE » ne seront jamais effaçables"
    );
}

/// Une **arête** sur un cas sans clé n'est pas comptée : elle ne porte aucun contenu, donc elle ne
/// perd rien. Compter tout nœud d'un cas non ouvert ferait gonfler un compteur de « données
/// ineffaçables » avec des nœuds qui n'ont rien à effacer — et un compteur qui crie pour rien use
/// la confiance qu'on lui accorde.
#[test]
fn une_arete_sans_cle_ne_gonfle_pas_le_compteur() {
    let mut store = CaptureStore::new(TenantKey::derive(b"racine", "acme"));
    store.avec_conservation(Trousseau::new());

    // Deux actes (qui perdent leur sortie) puis un message entre eux (qui ne perd rien).
    store.record_llm(&appel2("OUBLIE", "a", "PASS", 0)).unwrap();
    store.record_llm(&appel2("OUBLIE", "b", "PASS", 1)).unwrap();
    store
        .record_message(&provenance_capture::MessageEvent {
            case_id: "OUBLIE",
            step: "msg",
            from_step: "a",
            to_step: "b",
            artifact: b"charge",
            artifact_type: "json",
            timestamp: 2,
        })
        .expect("ingestion de l'arête");

    assert_eq!(
        store.sans_cle(),
        &BTreeMap::from([("OUBLIE".to_string(), 2u64)]),
        "deux ACTES perdent leur sortie ; l'arête ne perd rien et ne doit pas être comptée"
    );
    assert_eq!(store.records().len(), 3, "mais les trois nœuds sont bien scellés");
}

/// La posture par défaut ne change rien. Un binaire construit sans activer la conservation scelle
/// exactement les mêmes octets qu'avant M6 — c'est ce qui autorise M6 à exister sans migration.
#[test]
fn sans_conservation_la_chaine_est_identique() {
    let k = TenantKey::derive(b"racine", "acme");
    let mut avant = CaptureStore::new(k.clone());
    avant.record_llm(&appel("D-1", "COMPLIANT", 0)).unwrap();

    let mut apres = CaptureStore::new(k);
    apres.avec_conservation(Trousseau::new()); // activée, mais aucun cas ouvert
    apres.record_llm(&appel("D-1", "COMPLIANT", 0)).unwrap();

    assert_eq!(
        avant.reprove().manifest_root,
        apres.reprove().manifest_root,
        "sans clé de cas, le scellement est bit à bit celui d'avant M6"
    );
    assert!(avant.sans_cle().is_empty(), "et hors conservation, on ne compte rien");
}

/// Enrober deux fois n'est pas un aller-retour : le second passage chiffre le chiffré. Rien ne
/// l'interdit techniquement, et c'est pour cela qu'il faut le dire — la valeur redevient lisible
/// après DEUX ouvertures, pas une.
#[test]
fn enrober_nest_pas_idempotent_et_le_test_le_fige() {
    let c = CaseKey::from_bytes([1u8; 32]);
    let n = acte("C::x", "PASS");
    let une = enrober(n.clone(), &c);
    let deux = enrober(une.clone(), &c);
    assert_ne!(une, deux, "le second enrobage chiffre l'enveloppe du premier");
    let Node::AgentAct { output, .. } = &deux else { panic!() };
    assert!(est_enveloppe(output));
    // Et la re-preuve tient dans les deux cas : la chaîne ne juge pas le contenu.
    let k = TenantKey::derive(b"r", "t");
    let mut d = provenance_dag::SealedDag::new();
    d.append(&k, deux).unwrap();
    assert!(reprove_dag(d.records(), &k).ok);
}

// ── outillage ──

/// La portion de CODE d'une fonction — commentaires retirés.
///
/// Le retrait n'est pas cosmétique. La première version de ce garde est tombée sur le commentaire
/// qui explique *pourquoi* il n'y a pas de joker (« vaut mieux qu'un `_ => noeud` qui… ») : un garde
/// textuel qui lit la prose se trompe **dans les deux sens** — il peut refuser du code correct, et il
/// peut se laisser satisfaire par un commentaire qui décrit du code absent.
fn extraire(src: &str, debut: &str) -> String {
    let i = src.find(debut).unwrap_or_else(|| panic!("`{debut}` introuvable"));
    let reste = &src[i..];
    let fin = reste[1..].find("\n}\n").map_or(reste.len(), |j| j + 3);
    reste[..fin]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn acte(id: &str, sortie: &str) -> Node {
    Node::AgentAct {
        node_id: id.into(),
        agent_role: "r".into(),
        inputs_hash: "h".into(),
        model: "m".into(),
        version: "1".into(),
        output: sortie.into(),
        evidence_refs: vec![],
        timestamp: 1,
    }
}

fn appel<'a>(cas: &'a str, sortie: &'a str, t: u64) -> LlmCall<'a> {
    appel2(cas, "verdict", sortie, t)
}

fn appel2<'a>(cas: &'a str, etape: &'a str, sortie: &'a str, t: u64) -> LlmCall<'a> {
    LlmCall {
        case_id: cas,
        step: etape,
        agent_role: "verdict_aggregate",
        inputs: b"entrees",
        model: "phi4",
        version: "1",
        output: sortie,
        evidence_steps: &[],
        timestamp: t,
    }
}
