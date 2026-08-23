// SPDX-License-Identifier: UNLICENSED
//! # M6 vu du VÉRIFIEUR TIERS — le broyage lui est invisible, et c'est tout l'argument.
//!
//! La revendication du produit est qu'un tiers re-prouve un reçu **sans nous croire** : il recompile
//! `verify-mini`, dont l'arbre de dépendances est littéralement vide, et refait les mathématiques.
//!
//! Le crypto-shredding de M6 mettrait cette revendication en danger s'il touchait à ce que le
//! vérifieur lit. Une chaîne dont un cas a été effacé et qui ne serait re-prouvable que par nous
//! serait une régression pire que l'absence d'effacement : la promesse tomberait **précisément** le
//! jour où l'on exerce un droit.
//!
//! Elle ne tombe pas, parce que le vérifieur prouve l'**intégrité** et que le broyage retire la
//! **confidentialité** — deux propriétés orthogonales. Le vérifieur recalcule un HMAC sur des champs
//! dont il n'a jamais eu à comprendre le contenu. Ce fichier l'exige :
//!
//! 1. la sortie chiffrée traverse le parseur fait-main **sans que verify-mini n'ait de clé de cas** ;
//! 2. le verdict et le manifest-root sont **identiques** avant et après le broyage ;
//! 3. et le clair **n'est jamais sorti** — il n'est pas dans les octets publiés.
//!
//! ⚠ Un test qui affirme « rien n'a changé » ne vaut rien s'il ne prouve pas qu'il s'est passé
//! quelque chose. Le point (2) est donc encadré par une démonstration que le broyage a bien détruit
//! le contenu — sans quoi ce fichier passerait même si `broyer` ne faisait rien.

use provenance_dag::shred::{sceller, CaseKey, Lecture, Trousseau};
use provenance_dag::{reprove_dag, HumanAction, Node, SealedDag, TenantKey};

const KEY: [u8; 32] = [7u8; 32];
const CAS: &str = "SUP-2024";
/// Le clair qui ne doit jamais quitter la maison. Contient des pièges d'encodage, parce que le
/// parseur fait main est le point où une ré-implémentation diverge.
const SECRET: &str = "refus : \"ACME\" — pièces \\manquantes\\ 😀\nligne2";

fn cle_cas() -> CaseKey {
    CaseKey::from_bytes([42u8; 32])
}

/// Un DAG dont les champs effaçables sont scellés **chiffrés**, comme la capture les produit.
fn dag_enrobe() -> SealedDag {
    let k = TenantKey::from_bytes(KEY);
    let c = cle_cas();
    let mut d = SealedDag::new();

    d.append(&k, Node::AgentAct {
        node_id: format!("{CAS}::verdict"),
        agent_role: "verdict_aggregate".into(),
        inputs_hash: "h".into(),
        model: "phi4".into(),
        version: "1".into(),
        output: sceller(&c, &format!("{CAS}::verdict:output"), SECRET),
        evidence_refs: vec![],
        timestamp: 1,
    })
    .unwrap();

    d.append(&k, Node::Human {
        node_id: format!("{CAS}::revue"),
        human_id: sceller(&c, &format!("{CAS}::revue:human_id"), "alice@exemple"),
        action: HumanAction::Escalate,
        target_node: format!("{CAS}::verdict"),
        justification: sceller(&c, &format!("{CAS}::revue:justification"), SECRET),
        timestamp: 2,
    })
    .unwrap();

    d
}

#[test]
fn m6_le_verifieur_tiers_reprouve_a_lidentique_apres_broyage() {
    let dag = dag_enrobe();
    let jsonl = dag.to_jsonl();

    // (1) Le chiffré traverse le parseur fait-main. verify-mini n'a QUE la clé tenant : aucune clé
    //     de cas ne lui est confiée, et il n'en a jamais eu besoin.
    let avant = verify_mini::run(&jsonl, &KEY, None).expect("JSONL parsable");
    assert!(avant.ok, "le vérifieur tiers accepte une chaîne enrobée");
    let racine = avant.manifest_root.clone().expect("racine");
    assert_eq!(
        reprove_dag(dag.records(), &TenantKey::from_bytes(KEY)).manifest_root.as_ref(),
        Some(&racine),
        "S0 et le vérifieur zéro-dépendance concordent, comme sur une chaîne en clair"
    );

    // ⚠ Non-vacuité : on montre que le broyage DÉTRUIT vraiment, sinon l'égalité ci-dessous serait
    //   une tautologie satisfaite par un `broyer` qui ne ferait rien.
    let mut trousseau = Trousseau::new();
    trousseau.poser(CAS, cle_cas(), 0);
    let ctx = format!("{CAS}::verdict:output");
    let chiffre = match &dag.records()[0].node {
        Node::AgentAct { output, .. } => output.clone(),
        _ => panic!(),
    };
    assert_eq!(
        trousseau.lire(CAS, &ctx, &chiffre),
        Lecture::Ouvert(SECRET.into()),
        "avant : le contenu se lit"
    );
    assert!(trousseau.broyer(CAS, 999), "le broyage doit détruire quelque chose");
    assert_eq!(
        trousseau.lire(CAS, &ctx, &chiffre),
        Lecture::Broye { le: 999 },
        "après : le contenu est irrécupérable"
    );

    // (2) Et pour le vérifieur, rien n'a bougé. Même octets, même verdict, même racine.
    let apres = verify_mini::run(&jsonl, &KEY, None).expect("JSONL toujours parsable");
    assert!(apres.ok, "un cas broyé ne casse pas la re-preuve tierce");
    assert_eq!(apres.manifest_root, avant.manifest_root, "manifest-root inchangé");
    assert_eq!(apres.n_records, avant.n_records, "aucun record retiré : la chaîne reste entière");
    assert!(apres.broken_at.is_none());

    // Et contre le root publié — le mode dont se sert un auditeur qui tient une ancre.
    assert!(
        verify_mini::run(&jsonl, &KEY, Some(&racine)).unwrap().ok,
        "concordance avec le root publié, après broyage comme avant"
    );
}

/// Le clair ne sort jamais. C'est la confidentialité énoncée sur les octets qui quittent la maison,
/// et non sur une intention.
#[test]
fn m6_le_clair_nest_pas_dans_les_octets_publies() {
    let jsonl = dag_enrobe().to_jsonl();
    assert!(!jsonl.contains("refus"), "le clair de la décision ne doit pas figurer dans le bundle");
    assert!(!jsonl.contains("alice@exemple"), "ni l'identité de la personne");
    assert!(jsonl.contains("shred:v1:"), "mais l'enveloppe est RECONNAISSABLE — soustrait ≠ absurde");
}

/// Une chaîne enrobée reste falsifiable-détectable : altérer un chiffré casse le sceau, exactement
/// comme altérer un clair. Le chiffrement n'achète pas d'intégrité — la chaîne la fournit déjà — mais
/// il ne doit pas non plus en retirer.
#[test]
fn m6_alterer_un_chiffre_est_toujours_detecte() {
    let jsonl = dag_enrobe().to_jsonl();
    // On retourne un chiffre hexadécimal de l'enveloppe : le contenu reste illisible pour tous, et
    // c'est bien le SCEAU qui doit protester.
    let i = jsonl.find("shred:v1:").expect("enveloppe") + "shred:v1:".len();
    let mut altere = jsonl.clone();
    let remplacant = if &jsonl[i..i + 1] == "a" { "b" } else { "a" };
    altere.replace_range(i..i + 1, remplacant);

    let v = verify_mini::run(&altere, &KEY, None).expect("toujours parsable");
    assert!(!v.ok, "un chiffré altéré doit être refusé");
    assert_eq!(v.broken_at, Some(0), "au premier record, là où l'altération a eu lieu");
}
