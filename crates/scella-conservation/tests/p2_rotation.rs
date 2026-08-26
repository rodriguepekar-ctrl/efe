// SPDX-License-Identifier: UNLICENSED
//! **P2 — la rotation** : ce que devient un reçu scellé sous une clé retirée, puis détruite.
//!
//! C'est la question que la feuille de route posait sans y répondre, et elle a deux réponses
//! opposées selon un mot :
//!
//! * **retirée** — la clé ne scelle plus, mais elle re-prouve encore. Une archive ancienne se
//!   restaure **sans rien faire de particulier**. C'est ce qui rend une rotation praticable.
//! * **détruite** — la clé n'est plus. L'archive ne se re-prouvera plus, et doit se lire
//!   « détruite le … », **jamais** « invalide » : la seconde formulation accuserait quelqu'un
//!   d'avoir altéré la preuve, ce qui serait faux.

use std::path::PathBuf;

use provenance_dag::{Node, SealedDag, TenantKey};
use scella_conservation::archive::{sauvegarder, Prise};
use scella_conservation::{restaurer_avec, EtatGeneration, Issue, TrousseauMaitre};

fn dossier_neuf(nom: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("scella-p2rot-{}-{}", nom, std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("dossier créable");
    p
}

fn chaine(cle: &TenantKey, marque: &str) -> SealedDag {
    let mut d = SealedDag::new();
    d.append(
        cle,
        Node::AgentAct {
            node_id: format!("CAS::{marque}"),
            agent_role: "r".into(),
            inputs_hash: "h".into(),
            model: "m".into(),
            version: "1".into(),
            output: "PASS".into(),
            evidence_refs: vec![],
            timestamp: 1,
        },
    )
    .unwrap();
    d
}

fn prise(generation: u32) -> Prise {
    Prise {
        tenant: "acme".into(),
        cree_a: 1_000,
        generation,
        empreinte_trousseau: None,
        ancre: None,
    }
}

#[test]
fn apres_rotation_l_archive_ancienne_se_restaure_encore() {
    let g1 = TenantKey::from_bytes([1u8; 32]);
    let g2 = TenantKey::from_bytes([2u8; 32]);

    let mut t = TrousseauMaitre::new();
    t.poser(1, g1.clone(), 100).unwrap();

    // Une archive prise du temps de la génération 1.
    let vieille = dossier_neuf("g1");
    sauvegarder(chaine(&g1, "ancien").records(), &g1, &prise(1), &vieille).unwrap();

    // On fait tourner la clé.
    t.tourner(2, g2.clone(), 200).unwrap();
    assert_eq!(t.etat(1), EtatGeneration::Retiree { le: 200 });

    // L'archive ancienne se restaure SANS RIEN FAIRE DE PARTICULIER. C'est tout l'enjeu.
    let r = restaurer_avec(&vieille, &t);
    assert!(r.issue.est_restauree(), "après rotation, l'ancien doit encore remonter : {}", r.issue);

    // Et la nouvelle génération scelle bien le neuf.
    let neuve = dossier_neuf("g2");
    sauvegarder(chaine(&g2, "neuf").records(), &g2, &prise(2), &neuve).unwrap();
    assert!(restaurer_avec(&neuve, &t).issue.est_restauree());

    let _ = std::fs::remove_dir_all(&vieille);
    let _ = std::fs::remove_dir_all(&neuve);
}

#[test]
fn une_generation_detruite_se_lit_detruite_et_datee_jamais_invalide() {
    let g1 = TenantKey::from_bytes([1u8; 32]);
    let mut t = TrousseauMaitre::new();
    t.poser(1, g1.clone(), 100).unwrap();

    let arch = dossier_neuf("detruite");
    sauvegarder(chaine(&g1, "x").records(), &g1, &prise(1), &arch).unwrap();

    t.detruire(1, 999).unwrap();

    let r = restaurer_avec(&arch, &t);
    assert_eq!(
        r.issue,
        Issue::GenerationDetruite { generation: 1, le: 999 },
        "une destruction volontaire n'est pas une altération"
    );
    let msg = r.issue.to_string();
    assert!(msg.contains("999"), "la DATE doit atteindre l'exploitant : {msg}");
    assert!(!msg.contains("invalide"), "ne jamais accuser : {msg}");
    let _ = std::fs::remove_dir_all(&arch);
}

#[test]
fn une_generation_inconnue_n_affirme_pas_l_inexistence() {
    let g1 = TenantKey::from_bytes([1u8; 32]);
    let arch = dossier_neuf("inconnue");
    sauvegarder(chaine(&g1, "x").records(), &g1, &prise(42), &arch).unwrap();

    // Un trousseau qui ne connaît pas la génération 42 — par exemple celui d'une autre installation.
    let mut t = TrousseauMaitre::new();
    t.poser(1, g1, 100).unwrap();

    let r = restaurer_avec(&arch, &t);
    assert_eq!(r.issue, Issue::GenerationInconnue { generation: 42 });
    assert!(
        r.issue.to_string().contains("autre coffre"),
        "le message doit ouvrir la piste, pas clore le dossier : {}",
        r.issue
    );
    let _ = std::fs::remove_dir_all(&arch);
}

#[test]
fn une_mauvaise_generation_annoncee_est_prouvee_et_pas_supposee() {
    let g1 = TenantKey::from_bytes([1u8; 32]);
    let g2 = TenantKey::from_bytes([2u8; 32]);

    // L'archive est scellée par g1, mais son manifeste annonce — à tort — la génération 2.
    let arch = dossier_neuf("mauvaise-gen");
    sauvegarder(chaine(&g1, "x").records(), &g1, &prise(2), &arch).unwrap();

    let mut t = TrousseauMaitre::new();
    t.poser(1, g1, 100).unwrap();
    t.tourner(2, g2, 200).unwrap();

    let r = restaurer_avec(&arch, &t);
    assert_eq!(
        r.issue,
        Issue::MauvaiseGeneration { annoncee: 2, reelle: 1 },
        "quand une AUTRE génération re-prouve, ce n'est plus une hypothèse"
    );
    let _ = std::fs::remove_dir_all(&arch);
}

#[test]
fn sans_autre_generation_qui_reprouve_l_ambiguite_est_enoncee() {
    let vraie = TenantKey::from_bytes([1u8; 32]);
    let fausse = TenantKey::from_bytes([9u8; 32]);

    let arch = dossier_neuf("ambigu");
    sauvegarder(chaine(&vraie, "x").records(), &vraie, &prise(1), &arch).unwrap();

    // Le trousseau ne contient QUE la mauvaise clé sous le numéro annoncé.
    let mut t = TrousseauMaitre::new();
    t.poser(1, fausse, 100).unwrap();

    let r = restaurer_avec(&arch, &t);
    assert_eq!(r.issue, Issue::CleOuPremierRecord);
    let msg = r.issue.to_string();
    assert!(msg.contains("OU"), "les deux causes doivent être nommées : {msg}");
    let _ = std::fs::remove_dir_all(&arch);
}
