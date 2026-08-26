// SPDX-License-Identifier: UNLICENSED
//! **P2 — la confrontation à ce qui ne voyage pas avec l'archive.**
//!
//! La première version de ce crate vérifiait qu'une archive était cohérente **avec elle-même**.
//! C'est utile contre la corruption et contre l'adversaire négligent — et parfaitement inutile
//! contre celui qui réécrit les deux fichiers ensemble, puisqu'il détient les deux.
//!
//! Le test décisif de ce fichier est [`l_ancre_attrape_le_re_scellement_que_le_manifeste_laisse_passer`] :
//! il fabrique exactement cet adversaire — records re-scellés ET manifeste réécrit pour concorder —
//! puis montre que le diagnostic « racine divergente » ne se déclenche PAS, et que seule l'ancre
//! publiée l'attrape.

use std::path::PathBuf;

use provenance_dag::{AnchorRecord, Node, SealedDag, TenantKey};
use scella_conservation::archive::{sauvegarder, Prise};
use scella_conservation::manifeste::{Manifeste, NOM_MANIFESTE, NOM_RECORDS};
use scella_conservation::{restaurer_controle, Controle, Issue, TrousseauMaitre};

const RUN: &str = "run-2026-08-27-a";

fn dossier_neuf(nom: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("scella-p2conf-{}-{}", nom, std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("dossier créable");
    p
}

fn noeud(i: u64, sortie: &str) -> Node {
    Node::AgentAct {
        node_id: format!("CAS::etape{i}"),
        agent_role: "sanction_screen".into(),
        inputs_hash: "h".into(),
        model: "m".into(),
        version: "1".into(),
        output: sortie.into(),
        evidence_refs: vec![],
        timestamp: i,
    }
}

/// La chaîne « honnête » : trois étapes dont un criblage qui PASSE.
fn chaine_honnete(cle: &TenantKey) -> SealedDag {
    let mut d = SealedDag::new();
    d.append(cle, noeud(1, "intake")).unwrap();
    d.append(cle, noeud(2, "PASS")).unwrap();
    d.append(cle, noeud(3, "agregat")).unwrap();
    d
}

fn prise(ancre: Option<&str>) -> Prise {
    Prise {
        tenant: "acme".into(),
        cree_a: 1_000,
        generation: 1,
        empreinte_trousseau: None,
        ancre: ancre.map(str::to_string),
    }
}

fn trousseau(cle: &TenantKey) -> TrousseauMaitre {
    let mut t = TrousseauMaitre::new();
    t.poser(1, cle.clone(), 100).unwrap();
    t
}

/// Un journal d'ancres qui engage `racine` et `compte` pour [`RUN`].
fn journal(racine: &str, compte: u32) -> String {
    format!("{}\n", AnchorRecord::new(RUN, "acme", racine, compte, 1_000).unwrap().to_line())
}

#[test]
fn une_ancre_concordante_laisse_passer() {
    let cle = TenantKey::from_bytes([1u8; 32]);
    let d = chaine_honnete(&cle);
    let racine = d.manifest_root();
    let arch = dossier_neuf("ancre-ok");
    sauvegarder(d.records(), &cle, &prise(Some(RUN)), &arch).unwrap();

    let j = journal(&racine, 3);
    let c = Controle { journal_ancres: Some(&j), trousseau_cas: None };
    let r = restaurer_controle(&arch, &trousseau(&cle), &c);
    assert!(r.issue.est_restauree(), "{}", r.issue);
    let _ = std::fs::remove_dir_all(&arch);
}

#[test]
fn l_ancre_attrape_le_re_scellement_que_le_manifeste_laisse_passer() {
    // ── L'ADVERSAIRE DÉCIDÉ ───────────────────────────────────────────────────────────────
    // Il détient la clé. Il retourne le verdict de criblage PASS → FAIL, re-scelle TOUTE la
    // chaîne, puis — c'est le point que la première version de ce crate avait manqué — réécrit
    // aussi le manifeste pour qu'il concorde. Les deux fichiers sont à lui.
    let cle = TenantKey::from_bytes([1u8; 32]);
    let honnete = chaine_honnete(&cle);
    let racine_honnete = honnete.manifest_root();

    let arch = dossier_neuf("re-scelle");
    sauvegarder(honnete.records(), &cle, &prise(Some(RUN)), &arch).unwrap();

    let mut menteuse = SealedDag::new();
    menteuse.append(&cle, noeud(1, "intake")).unwrap();
    menteuse.append(&cle, noeud(2, "FAIL")).unwrap(); // le mensonge
    menteuse.append(&cle, noeud(3, "agregat")).unwrap();
    std::fs::write(arch.join(NOM_RECORDS), format!("{}\n", menteuse.to_jsonl())).unwrap();

    // Le manifeste, réécrit pour concorder — le geste qui neutralise `RacineDivergente`.
    let mut m = Manifeste::lire(&std::fs::read_to_string(arch.join(NOM_MANIFESTE)).unwrap()).unwrap();
    m.racine = menteuse.manifest_root();
    std::fs::write(arch.join(NOM_MANIFESTE), m.rendre().unwrap()).unwrap();

    // ── 1. SANS ancre : l'archive est cohérente avec elle-même. Rien ne se déclenche. ──────
    let sans = restaurer_controle(&arch, &trousseau(&cle), &Controle::default());
    assert!(
        sans.issue.est_restauree(),
        "sans ancre, le re-scellement passe — c'est précisément la limite : {}",
        sans.issue
    );

    // ── 2. AVEC l'ancre publiée AVANT le mensonge : il est attrapé. ────────────────────────
    let j = journal(&racine_honnete, 3);
    let c = Controle { journal_ancres: Some(&j), trousseau_cas: None };
    let avec = restaurer_controle(&arch, &trousseau(&cle), &c);
    match &avec.issue {
        Issue::AncreDivergente { run_id, .. } => assert_eq!(run_id, RUN),
        autre => panic!("l'ancre devait attraper le re-scellement, obtenu : {autre}"),
    }
    assert!(
        avec.issue.to_string().contains("l'ancre non"),
        "le message doit dire POURQUOI l'ancre l'emporte : {}",
        avec.issue
    );
    let _ = std::fs::remove_dir_all(&arch);
}

#[test]
fn une_troncature_est_attrapee_par_l_ancre_meme_manifeste_recrit() {
    // Variante du même adversaire : il tronque et rajuste le compte du manifeste.
    let cle = TenantKey::from_bytes([1u8; 32]);
    let d = chaine_honnete(&cle);
    let racine = d.manifest_root();
    let arch = dossier_neuf("tronque-ajuste");
    sauvegarder(d.records(), &cle, &prise(Some(RUN)), &arch).unwrap();

    let contenu = std::fs::read_to_string(arch.join(NOM_RECORDS)).unwrap();
    let gardees: Vec<&str> = contenu.lines().take(2).collect();
    std::fs::write(arch.join(NOM_RECORDS), format!("{}\n", gardees.join("\n"))).unwrap();
    let mut m = Manifeste::lire(&std::fs::read_to_string(arch.join(NOM_MANIFESTE)).unwrap()).unwrap();
    m.noeuds = 2;
    let mut prefixe = SealedDag::new();
    prefixe.append(&cle, noeud(1, "intake")).unwrap();
    prefixe.append(&cle, noeud(2, "PASS")).unwrap();
    m.racine = prefixe.manifest_root();
    std::fs::write(arch.join(NOM_MANIFESTE), m.rendre().unwrap()).unwrap();

    assert!(
        restaurer_controle(&arch, &trousseau(&cle), &Controle::default()).issue.est_restauree(),
        "sans ancre, une troncature ajustée passe"
    );

    let j = journal(&racine, 3);
    let c = Controle { journal_ancres: Some(&j), trousseau_cas: None };
    let r = restaurer_controle(&arch, &trousseau(&cle), &c);
    match &r.issue {
        Issue::AncreDivergente { motif, .. } => {
            assert!(motif.contains("nœuds") || motif.contains("noeuds"), "motif : {motif}")
        }
        autre => panic!("attendu AncreDivergente, obtenu {autre}"),
    }
    let _ = std::fs::remove_dir_all(&arch);
}

#[test]
fn une_ancre_absente_du_journal_n_accuse_personne() {
    let cle = TenantKey::from_bytes([1u8; 32]);
    let arch = dossier_neuf("ancre-absente");
    sauvegarder(chaine_honnete(&cle).records(), &cle, &prise(Some(RUN)), &arch).unwrap();

    let j = journal("f".repeat(64).as_str(), 3).replace(RUN, "un-autre-run");
    let c = Controle { journal_ancres: Some(&j), trousseau_cas: None };
    let r = restaurer_controle(&arch, &trousseau(&cle), &c);
    assert_eq!(r.issue, Issue::AncreAbsente { run_id: RUN.into() });
    assert!(
        r.issue.to_string().contains("pas une accusation"),
        "un journal partiel n'est pas une preuve de fraude : {}",
        r.issue
    );
    let _ = std::fs::remove_dir_all(&arch);
}

#[test]
fn un_journal_illisible_se_distingue_d_une_ancre_absente() {
    let cle = TenantKey::from_bytes([1u8; 32]);
    let arch = dossier_neuf("journal-casse");
    sauvegarder(chaine_honnete(&cle).records(), &cle, &prise(Some(RUN)), &arch).unwrap();

    let c = Controle { journal_ancres: Some("ceci n'est pas un journal"), trousseau_cas: None };
    let r = restaurer_controle(&arch, &trousseau(&cle), &c);
    assert!(matches!(r.issue, Issue::AncreIllisible { .. }), "{}", r.issue);
    let _ = std::fs::remove_dir_all(&arch);
}

#[test]
fn sans_journal_fourni_l_ancre_n_est_pas_exigee() {
    // Une machine de secours n'a que l'archive et la clé. Exiger le journal en ferait un obstacle
    // au lieu d'un renfort.
    let cle = TenantKey::from_bytes([1u8; 32]);
    let arch = dossier_neuf("hors-ligne");
    sauvegarder(chaine_honnete(&cle).records(), &cle, &prise(Some(RUN)), &arch).unwrap();
    let r = restaurer_controle(&arch, &trousseau(&cle), &Controle::default());
    assert!(r.issue.est_restauree(), "{}", r.issue);
    let _ = std::fs::remove_dir_all(&arch);
}

#[test]
fn un_trousseau_de_cas_different_est_signale() {
    let cle = TenantKey::from_bytes([1u8; 32]);
    let base = dossier_neuf("trousseau");
    let vrai = base.join("trousseau.vrai");
    let autre = base.join("trousseau.autre");
    std::fs::write(&vrai, "SUP-2024\t3f9a\t100\n").unwrap();
    std::fs::write(&autre, "SUP-2024\tCEQUINEVAPAS\t100\n").unwrap();

    let arch = base.join("archive");
    let mut p = prise(None);
    p.empreinte_trousseau =
        Some(scella_conservation::archive::empreinte_fichier(&vrai).unwrap());
    sauvegarder(chaine_honnete(&cle).records(), &cle, &p, &arch).unwrap();

    // Le bon trousseau : rien à signaler.
    let bon = Controle { journal_ancres: None, trousseau_cas: Some(&vrai) };
    assert!(restaurer_controle(&arch, &trousseau(&cle), &bon).issue.est_restauree());

    // Un autre : la chaîne se re-prouve quand même, et pourtant il faut le dire.
    let mauvais = Controle { journal_ancres: None, trousseau_cas: Some(&autre) };
    let r = restaurer_controle(&arch, &trousseau(&cle), &mauvais);
    assert!(matches!(r.issue, Issue::TrousseauDivergent { .. }), "{}", r.issue);
    assert!(
        r.issue.to_string().contains("clé absente"),
        "le message doit annoncer le symptôme que l'exploitant VERRA : {}",
        r.issue
    );

    // Aucun trousseau présenté, alors que le manifeste en attend un.
    let r2 = restaurer_controle(&arch, &trousseau(&cle), &Controle::default());
    assert!(matches!(r2.issue, Issue::TrousseauNonPresente { .. }), "{}", r2.issue);
    let _ = std::fs::remove_dir_all(&base);
}
