// SPDX-License-Identifier: UNLICENSED
//! **P2 — le critère de la feuille de route, exécuté** : « un store détruit puis restauré, dont la
//! chaîne se re-prouve ».
//!
//! Le mot qui compte est **détruit**. Un test qui archiverait puis relirait sans rien supprimer ne
//! prouverait que la sérialisation. Ici le dossier du store est effacé du disque entre les deux, et
//! la restauration ne peut donc s'appuyer que sur l'archive.
//!
//! La re-preuve par le **binaire** `verify-mini` — le vérifieur à dépendances nulles qu'un tiers
//! recompile — est faite par `scripts/p2-check.sh`, parce qu'elle demande de lancer un exécutable.
//! Ici on exerce la même arithmétique par `reprove_dag`.

use std::path::PathBuf;

use provenance_dag::{reprove_dag, Node, SealedDag, TenantKey};
use scella_conservation::archive::{sauvegarder, Prise};
use scella_conservation::restaurer;

fn dossier_neuf(nom: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("scella-p2-{}-{}", nom, std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("dossier de test créable");
    p
}

fn acte(id: &str, sortie: &str, t: u64) -> Node {
    Node::AgentAct {
        node_id: id.into(),
        agent_role: "sanction_screen".into(),
        inputs_hash: "h".into(),
        model: "phi4".into(),
        version: "1".into(),
        output: sortie.into(),
        evidence_refs: vec![],
        timestamp: t,
    }
}

/// Une chaîne de diligence, scellée — la matière de tous les tests de ce fichier.
fn chaine(cle: &TenantKey) -> SealedDag {
    let mut d = SealedDag::new();
    d.append(cle, acte("SUP-2024::intake", "supplier=Nordwind", 1)).unwrap();
    d.append(cle, acte("SUP-2024::sanction", "PASS", 2)).unwrap();
    d.append(cle, acte("SUP-2024::esg", "REVIEW", 3)).unwrap();
    d.append(cle, acte("SUP-2024::agregat", "CONDITIONNEL", 4)).unwrap();
    d
}

fn prise() -> Prise {
    Prise {
        tenant: "acme-corp".into(),
        cree_a: 1_756_166_400,
        generation: 1,
        empreinte_trousseau: None,
        ancre: None,
    }
}

#[test]
fn un_store_detruit_puis_restaure_se_reprouve() {
    let cle = TenantKey::from_bytes([7u8; 32]);
    let d = chaine(&cle);
    let racine_avant = d.manifest_root();

    // ── 1. Le store vit sur le disque, comme en production.
    let store = dossier_neuf("store");
    std::fs::write(store.join("records.jsonl"), d.to_jsonl()).unwrap();

    // ── 2. On archive.
    let arch = dossier_neuf("archive");
    let sauvegarde = sauvegarder(d.records(), &cle, &prise(), &arch).expect("archivage");
    assert_eq!(sauvegarde.manifeste.racine, racine_avant);
    assert_eq!(sauvegarde.manifeste.noeuds, 4);

    // ── 3. LE STORE EST DÉTRUIT. C'est tout l'objet du test.
    std::fs::remove_dir_all(&store).unwrap();
    assert!(!store.exists(), "le store n'est plus là");

    // ── 4. On restaure depuis la seule archive.
    let r = restaurer(&arch, &cle);
    assert!(r.issue.est_restauree(), "issue inattendue : {}", r.issue);
    assert_eq!(r.records.len(), 4);

    // ── 5. Et la chaîne restaurée se re-prouve, avec la MÊME racine qu'avant la destruction.
    let v = reprove_dag(&r.records, &cle);
    assert!(v.ok, "la chaîne restaurée doit se re-prouver");
    assert_eq!(v.manifest_root.as_deref(), Some(racine_avant.as_str()));

    let _ = std::fs::remove_dir_all(&arch);
}

#[test]
fn l_archive_ne_contient_jamais_la_cle() {
    // La clé est faite d'octets reconnaissables : si un jour quelqu'un la sérialisait « pour
    // simplifier la restauration », sa forme hexadécimale apparaîtrait dans un des fichiers.
    let cle = TenantKey::from_bytes([0xAB; 32]);
    let arch = dossier_neuf("sans-cle");
    sauvegarder(chaine(&cle).records(), &cle, &prise(), &arch).unwrap();

    let hex_cle = "ab".repeat(32);
    for f in ["records.jsonl", "MANIFESTE"] {
        let contenu = std::fs::read_to_string(arch.join(f)).unwrap();
        assert!(
            !contenu.to_lowercase().contains(&hex_cle),
            "{f} contient la clé — l'invariant « une archive ne porte pas sa clé » est rompu"
        );
    }
    let _ = std::fs::remove_dir_all(&arch);
}

#[test]
fn on_refuse_d_archiver_une_chaine_rompue() {
    let cle = TenantKey::from_bytes([7u8; 32]);
    let mut records = chaine(&cle).records().to_vec();
    // Un sceau forgé au milieu — exactement ce qu'une sauvegarde ne doit PAS graver.
    records[2].seal = "00".repeat(32);

    let arch = dossier_neuf("rompue");
    let e = sauvegarder(&records, &cle, &prise(), &arch).expect_err("l'archivage doit refuser");
    let msg = e.to_string();
    assert!(msg.contains("record 2"), "le refus doit NOMMER le record fautif : {msg}");
    assert!(
        !arch.join("MANIFESTE").exists(),
        "un refus ne doit laisser aucune archive à moitié écrite"
    );
    let _ = std::fs::remove_dir_all(&arch);
}

#[test]
fn une_archive_vide_est_legitime_et_le_dit() {
    // Un store neuf n'a rien à cacher : sa chaîne se re-prouve (racine = genèse). Refuser
    // l'archivage serait refuser un état parfaitement valide — et pousser l'exploitant à croire
    // que sa sauvegarde a échoué.
    let cle = TenantKey::from_bytes([3u8; 32]);
    let arch = dossier_neuf("vide");
    let s = sauvegarder(&[], &cle, &prise(), &arch).expect("un store vide s'archive");
    assert_eq!(s.manifeste.noeuds, 0);
    let r = restaurer(&arch, &cle);
    assert!(r.issue.est_restauree(), "{}", r.issue);
    assert!(r.records.is_empty());
    let _ = std::fs::remove_dir_all(&arch);
}
