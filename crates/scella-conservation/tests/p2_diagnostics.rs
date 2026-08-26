// SPDX-License-Identifier: UNLICENSED
//! **P2 — les neuf issues, chacune provoquée pour de vrai.**
//!
//! Un diagnostic qu'aucun test ne provoque est une promesse. Chaque cas ci-dessous fabrique
//! l'incident correspondant sur le disque, puis exige l'issue exacte — et, quand le message porte
//! l'information utile à l'exploitant (une date, un compte, une piste), exige aussi le message.

use std::path::PathBuf;

use provenance_dag::{Node, SealedDag, TenantKey};
use scella_conservation::archive::{sauvegarder, Prise};
use scella_conservation::manifeste::{NOM_MANIFESTE, NOM_RECORDS};
use scella_conservation::{restaurer, Issue};

fn dossier_neuf(nom: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("scella-p2diag-{}-{}", nom, std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("dossier créable");
    p
}

fn noeud(i: u64) -> Node {
    Node::AgentAct {
        node_id: format!("CAS::etape{i}"),
        agent_role: "r".into(),
        inputs_hash: "h".into(),
        model: "m".into(),
        version: "1".into(),
        output: format!("sortie{i}"),
        evidence_refs: vec![],
        timestamp: i,
    }
}

fn chaine(cle: &TenantKey, n: u64) -> SealedDag {
    let mut d = SealedDag::new();
    for i in 1..=n {
        d.append(cle, noeud(i)).unwrap();
    }
    d
}

fn prise() -> Prise {
    Prise { tenant: "t".into(), cree_a: 1, generation: 1, empreinte_trousseau: None, ancre: None }
}

/// Archive une chaîne de `n` nœuds et rend (dossier, clé).
fn archive(nom: &str, n: u64) -> (PathBuf, TenantKey) {
    let cle = TenantKey::from_bytes([5u8; 32]);
    let d = dossier_neuf(nom);
    sauvegarder(chaine(&cle, n).records(), &cle, &prise(), &d).unwrap();
    (d, cle)
}

#[test]
fn restauree_quand_rien_ne_cloche() {
    let (d, cle) = archive("ok", 3);
    let r = restaurer(&d, &cle);
    assert!(matches!(r.issue, Issue::Restauree { noeuds: 3, .. }), "{}", r.issue);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn tronquee_quand_la_fin_a_disparu() {
    let (d, cle) = archive("tronquee", 5);
    // On retire les deux dernières lignes : la chaîne restante reste PARFAITEMENT valide.
    let contenu = std::fs::read_to_string(d.join(NOM_RECORDS)).unwrap();
    let gardees: Vec<&str> = contenu.lines().take(3).collect();
    std::fs::write(d.join(NOM_RECORDS), format!("{}\n", gardees.join("\n"))).unwrap();

    let r = restaurer(&d, &cle);
    assert_eq!(r.issue, Issue::Tronquee { attendus: 5, trouves: 3 });
    assert!(
        r.issue.to_string().contains("préfixe de chaîne valide"),
        "le message doit expliquer POURQUOI la re-preuve seule ne l'attrape pas : {}",
        r.issue
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn etendue_quand_l_archive_a_vecu_apres_sa_prise() {
    let cle = TenantKey::from_bytes([5u8; 32]);
    let d = dossier_neuf("etendue");
    // Le manifeste est pris à 2 nœuds, mais le fichier en porte 4 : la chaîne a continué.
    sauvegarder(chaine(&cle, 2).records(), &cle, &prise(), &d).unwrap();
    std::fs::write(d.join(NOM_RECORDS), format!("{}\n", chaine(&cle, 4).to_jsonl())).unwrap();

    let r = restaurer(&d, &cle);
    assert_eq!(r.issue, Issue::Etendue { attendus: 2, trouves: 4 });
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn chaine_rompue_quand_un_record_du_milieu_est_altere() {
    let (d, cle) = archive("rompue", 4);
    let contenu = std::fs::read_to_string(d.join(NOM_RECORDS)).unwrap();
    let mut lignes: Vec<String> = contenu.lines().map(str::to_string).collect();
    // On change la SORTIE du 3e record sans toucher au sceau : la chaîne casse là, et pas avant.
    lignes[2] = lignes[2].replace("sortie3", "sortie3_ALTEREE");
    std::fs::write(d.join(NOM_RECORDS), format!("{}\n", lignes.join("\n"))).unwrap();

    let r = restaurer(&d, &cle);
    assert_eq!(r.issue, Issue::ChaineRompue { a: 2 }, "{}", r.issue);
    assert!(
        r.issue.to_string().contains("ce n'est pas la clé"),
        "au-delà du premier maillon, il faut écarter la clé explicitement : {}",
        r.issue
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn cle_ou_premier_record_quand_la_cle_est_fausse() {
    let (d, _) = archive("fausse-cle", 3);
    let r = restaurer(&d, &TenantKey::from_bytes([0xFF; 32]));
    assert_eq!(r.issue, Issue::CleOuPremierRecord);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn racine_divergente_quand_la_chaine_a_ete_re_scellee() {
    // Le cas que seule la racine attrape : le détenteur de la clé réécrit l'histoire et re-scelle
    // TOUT. La chaîne se re-prouve, la longueur est la même — seule la racine trahit.
    let (d, cle) = archive("re-scellee", 3);
    let mut refaite = SealedDag::new();
    refaite.append(&cle, noeud(1)).unwrap();
    refaite
        .append(
            &cle,
            Node::AgentAct {
                node_id: "CAS::etape2".into(),
                agent_role: "r".into(),
                inputs_hash: "h".into(),
                model: "m".into(),
                version: "1".into(),
                output: "VERDICT REECRIT".into(), // le mensonge
                evidence_refs: vec![],
                timestamp: 2,
            },
        )
        .unwrap();
    refaite.append(&cle, noeud(3)).unwrap();
    std::fs::write(d.join(NOM_RECORDS), format!("{}\n", refaite.to_jsonl())).unwrap();

    let r = restaurer(&d, &cle);
    match &r.issue {
        Issue::RacineDivergente { .. } => {}
        autre => panic!("attendu RacineDivergente, obtenu : {autre}"),
    }
    assert!(
        r.issue.to_string().contains("RE-SCELLÉE"),
        "le message doit nommer le geste, pas seulement l'écart : {}",
        r.issue
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn manifeste_illisible_quand_il_manque() {
    let (d, cle) = archive("sans-manifeste", 2);
    std::fs::remove_file(d.join(NOM_MANIFESTE)).unwrap();
    let r = restaurer(&d, &cle);
    assert!(matches!(r.issue, Issue::ManifesteIllisible { .. }), "{}", r.issue);
    assert!(r.manifeste.is_none());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn records_illisibles_quand_ils_manquent_mais_le_manifeste_est_rendu() {
    let (d, cle) = archive("sans-records", 2);
    std::fs::remove_file(d.join(NOM_RECORDS)).unwrap();
    let r = restaurer(&d, &cle);
    assert!(matches!(r.issue, Issue::RecordsIllisibles { .. }), "{}", r.issue);
    assert!(
        r.manifeste.is_some(),
        "le manifeste lu doit rester disponible : c'est lui qui dit ce qu'on cherche"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn le_compte_passe_avant_la_racine_dans_le_diagnostic() {
    // Quand les deux gardes tombent en même temps, c'est le COMPTE qu'on annonce : deux nombres se
    // comparent d'un coup d'œil, deux racines de 64 caractères non.
    let (d, cle) = archive("deux-gardes", 5);
    let contenu = std::fs::read_to_string(d.join(NOM_RECORDS)).unwrap();
    let gardees: Vec<&str> = contenu.lines().take(2).collect();
    std::fs::write(d.join(NOM_RECORDS), format!("{}\n", gardees.join("\n"))).unwrap();
    let r = restaurer(&d, &cle);
    assert!(matches!(r.issue, Issue::Tronquee { .. }), "le compte d'abord : {}", r.issue);
    let _ = std::fs::remove_dir_all(&d);
}
