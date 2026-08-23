// SPDX-License-Identifier: UNLICENSED
//! # M6, câblé au service — le critère, exécuté sur le chemin SERVI.
//!
//! ## Ce que l'audit M6→M8 a trouvé
//!
//! M6 avait livré le crypto-shredding : primitive, politique, point de scellement unique, façade
//! sur `AuditService`, critère prouvé, garde, exemple, script. Et **aucun binaire ne l'allumait**.
//! `grep -rn avec_conservation crates/*/src` ne trouvait rien : la façade existait, personne ne
//! l'appelait. C'est le trou « aucun producteur », que M7 avait fermé pour le provisionnement et que
//! l'audit de M5 avait trouvé pour le pont réglementaire — la troisième fois dans ce dépôt.
//!
//! Trois manques, qui n'en font qu'un :
//!
//! 1. rien n'**activait** la conservation ;
//! 2. le trousseau n'avait aucun chemin de **chargement**, alors que M7 avait bâti une
//!    réhydratation — un redémarrage rendait donc tout le contenu enrobé illisible, en silence ;
//! 3. rien n'**exécutait** la politique : un horizon déclaré n'aurait jamais fait expirer personne.
//!
//! ## Le critère de ce fichier
//!
//! > **Un cas expiré disparaît, la destruction atteint le disque, et elle survit au redémarrage
//! > AVEC SA DATE.**
//!
//! Cette dernière clause n'était pas dans M6, et c'est en écrivant ce test qu'elle est apparue :
//! une destruction durable qui efface son propre procès-verbal n'est pas durable. « Détruit le
//! 12 mars » redeviendrait « jamais vu ».

use std::path::PathBuf;

use audit_console::provisioning::{appliquer, balayer, Deploiement};
use audit_console::Console;
use provenance_capture::enrober;
use provenance_dag::shred::{CaseKey, EtatCas, Lecture};
use provenance_dag::{Node, SealedDag, TenantKey};

const RACINE: &[u8] = b"racine-de-test-m6-cable";
const HORIZON: u64 = 500;

/// Deux cas : un vieux (ouvert à 0) et un récent (ouvert à 900).
const CAS: [(&str, u64, &str); 2] = [("DOSSIER-VIEUX", 0, "COMPLIANT"), ("DOSSIER-RECENT", 900, "ESCALADE")];

fn cle_de(cas: &str) -> CaseKey {
    let mut b = [0u8; 32];
    for (i, o) in cas.bytes().enumerate().take(32) {
        b[i] = o;
    }
    CaseKey::from_bytes(b)
}

fn hex_de(cas: &str) -> String {
    let mut b = [0u8; 32];
    for (i, o) in cas.bytes().enumerate().take(32) {
        b[i] = o;
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn tmp(nom: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("scella-m6c-{nom}-{}", std::process::id()));
    p
}

/// Le décor : un log scellé dont les sorties sont **enrobées**, et le trousseau qui les ouvre.
fn decor(nom: &str) -> (PathBuf, PathBuf, PathBuf) {
    let k = TenantKey::derive(RACINE, "acme");
    let mut dag = SealedDag::new();
    for (cas, t, verdict) in CAS {
        let acte = Node::AgentAct {
            node_id: format!("{cas}::verdict"),
            agent_role: "verdict_aggregate".into(),
            inputs_hash: "0".repeat(64),
            model: "phi4".into(),
            version: "1".into(),
            output: verdict.into(),
            evidence_refs: vec![],
            timestamp: t,
        };
        dag.append(&k, enrober(acte, &cle_de(cas))).expect("acte");
    }

    let records = tmp(&format!("{nom}-records.jsonl"));
    std::fs::write(&records, dag.to_jsonl()).expect("records");

    let keys = tmp(&format!("{nom}.keys"));
    let mut contenu = String::from("# trousseau de vérification\n");
    for (cas, t, _) in CAS {
        contenu.push_str(&format!("{cas}\t{}\t{t}\n", hex_de(cas)));
    }
    std::fs::write(&keys, contenu).expect("trousseau");

    let decl = tmp(&format!("{nom}-decl.json"));
    std::fs::write(
        &decl,
        format!(
            r#"{{
              "tenants": [{{ "id": "acme",
                             "records": "{}",
                             "conservation": {{ "horizon_s": {HORIZON}, "trousseau": "{}" }} }}],
              "sessions": [{{ "token": "t", "tenant": "acme", "role": "auditor", "subject": "a" }}]
            }}"#,
            records.display(),
            keys.display()
        ),
    )
    .expect("déclaration");
    (records, keys, decl)
}

fn monter(decl: &PathBuf, maintenant: u64) -> (Console, Deploiement) {
    let brut = std::fs::read_to_string(decl).expect("déclaration lisible");
    let dep = Deploiement::depuis_json(&brut).expect("bien formée");
    let mut c = Console::new();
    appliquer(&dep, &mut c, RACINE, maintenant).expect("déclaration saine");
    (c, dep)
}

/// Lit la sortie d'un cas à travers le service **servi**.
fn lire(c: &mut Console, cas: &str) -> Lecture {
    let svc = c.service_de("acme").expect("tenant monté");
    let noeud = svc
        .store()
        .records()
        .iter()
        .map(|r| &r.node)
        .find(|n| n.id() == format!("{cas}::verdict"))
        .expect("nœud présent");
    let t = svc.store().conservation().expect("conservation active");
    provenance_capture::lire(noeud, t)["output"].clone()
}

fn menage(v: &[&PathBuf]) {
    for p in v {
        let _ = std::fs::remove_file(p);
    }
}

// ═══ LE CRITÈRE ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn m6_cable_un_cas_expire_disparait_et_la_destruction_survit_au_redemarrage() {
    let (records, keys, decl) = decor("critere");

    // ── Avant : la conservation est ACTIVE (elle ne l'était par aucun binaire), et tout se lit. ──
    let (mut c, dep) = monter(&decl, 0);
    assert_eq!(lire(&mut c, "DOSSIER-VIEUX"), Lecture::Ouvert("COMPLIANT".into()));
    assert_eq!(lire(&mut c, "DOSSIER-RECENT"), Lecture::Ouvert("ESCALADE".into()));

    let racine_avant = c.service_de("acme").unwrap().store().reprove().manifest_root.clone();

    // ── Le balayage : horizon 500, il est 1000. Seul le vieux a expiré. ──
    let rapports = balayer(&mut c, &dep, 1000);
    assert_eq!(rapports.len(), 1, "un rapport, pour le seul tenant qui a de la conservation");
    let r = &rapports[0];
    assert!(r.durable, "la destruction doit avoir atteint le disque : {:?}", r.echec);
    assert_eq!(r.broyes, vec!["DOSSIER-VIEUX".to_string()]);

    assert_eq!(lire(&mut c, "DOSSIER-VIEUX"), Lecture::Broye { le: 1000 }, "le contenu est parti");
    assert_eq!(lire(&mut c, "DOSSIER-RECENT"), Lecture::Ouvert("ESCALADE".into()), "l'autre survit");
    assert_eq!(
        c.service_de("acme").unwrap().store().reprove().manifest_root,
        racine_avant,
        "et la chaîne se re-prouve sur la MÊME racine — le critère de M6, sur le chemin servi"
    );

    // ── Sur le DISQUE : la clé n'y est plus, la date y est. ──
    let sur_disque = std::fs::read_to_string(&keys).expect("trousseau relu");
    assert!(!sur_disque.contains(&hex_de("DOSSIER-VIEUX")), "la clé a quitté le disque");
    assert!(sur_disque.contains("DOSSIER-VIEUX"), "le cas demeure");
    assert!(sur_disque.contains("1000"), "et la DATE de destruction avec lui");

    // ── ⭐ LE REDÉMARRAGE. Un processus neuf, la même déclaration. ──
    let (mut c2, _) = monter(&decl, 2000);
    assert_eq!(
        lire(&mut c2, "DOSSIER-VIEUX"),
        Lecture::Broye { le: 1000 },
        "après redémarrage : toujours BROYÉ, et à la MÊME date. Perdre la date ferait redevenir \
         « détruit le 12 mars » en « jamais vu » — ce que M6 existe pour empêcher"
    );
    assert_eq!(
        c2.service_de("acme").unwrap().etat_conservation("DOSSIER-VIEUX"),
        Some(EtatCas::Broye { le: 1000 })
    );
    assert_eq!(lire(&mut c2, "DOSSIER-RECENT"), Lecture::Ouvert("ESCALADE".into()));

    menage(&[&records, &keys, &decl]);
}

/// Un second balayage ne détruit rien de plus et ne réécrit aucune date.
#[test]
fn m6_cable_un_second_balayage_est_sans_effet() {
    let (records, keys, decl) = decor("idempotent");
    let (mut c, dep) = monter(&decl, 0);
    assert_eq!(balayer(&mut c, &dep, 1000)[0].broyes.len(), 1);
    // ⚠ t=1200 et non 2000 : à 2000 le dossier « récent » (ouvert à 900) aurait lui aussi 1100 s,
    //   donc franchi l'horizon de 500. La première version de ce test l'ignorait — le code avait
    //   raison, l'assertion se trompait.
    assert!(balayer(&mut c, &dep, 1200).is_empty(), "plus rien n'a expiré : aucun rapport");
    assert_eq!(lire(&mut c, "DOSSIER-VIEUX"), Lecture::Broye { le: 1000 }, "la date de la 1ʳᵉ fois");
    menage(&[&records, &keys, &decl]);
}

/// ⭐ **Un broyage qui n'atteint pas le disque n'a PAS lieu.** Rien n'est détruit, ni sur le disque
/// ni en mémoire — parce que broyer en mémoire pendant que les clés restent sur le disque
/// déclarerait détruit ce qui est intact, et le redémarrage suivant les ressusciterait.
#[test]
fn m6_cable_une_destruction_non_durable_nest_pas_appliquee_du_tout() {
    let (records, keys, decl) = decor("nondurable");
    let (mut c, dep) = monter(&decl, 0);

    // On rend le trousseau inaccessible : le fichier disparaît sous les pieds du balayage.
    std::fs::remove_file(&keys).expect("retiré");

    let rapports = balayer(&mut c, &dep, 1000);
    assert_eq!(rapports.len(), 1);
    let r = &rapports[0];
    assert!(!r.durable, "la destruction n'a pas atteint le disque");
    assert!(r.broyes.is_empty(), "donc RIEN n'a été broyé en mémoire non plus");
    assert_eq!(r.expires, vec!["DOSSIER-VIEUX".to_string()], "mais on DIT ce qui aurait dû l'être");
    assert!(r.echec.is_some());

    assert_eq!(
        lire(&mut c, "DOSSIER-VIEUX"),
        Lecture::Ouvert("COMPLIANT".into()),
        "le contenu se lit encore : mieux vaut un effacement en retard qu'un effacement menti"
    );
    menage(&[&records, &decl]);
}

/// Un horizon nul broierait tout le registre au premier balayage. Dans un fichier de configuration,
/// un zéro est une faute de frappe bien plus souvent qu'une intention.
#[test]
fn m6_cable_un_horizon_nul_arrete_le_demarrage() {
    let d = Deploiement::depuis_json(
        r#"{"tenants":[{"id":"a","conservation":{"horizon_s":0,"trousseau":"/x"}}],
            "sessions":[{"token":"t","tenant":"a","role":"auditor","subject":"s"}]}"#,
    )
    .expect("bien formée");
    let e = d.valider(0).expect_err("doit refuser");
    let msg = e.iter().map(|r| r.to_string()).collect::<String>();
    assert!(msg.contains("horizon de conservation NUL"), "{msg}");
    assert!(msg.contains("supprimez le fichier de trousseau"), "et le geste honnête : {msg}");
}

/// Un trousseau illisible **arrête le démarrage** : sans lui, tout le contenu enrobé serait
/// illisible et le broyage ne serait pas durable — deux pannes silencieuses pour le prix d'une.
#[test]
fn m6_cable_un_trousseau_illisible_arrete_le_demarrage() {
    let decl = tmp("absent-decl.json");
    std::fs::write(
        &decl,
        r#"{"tenants":[{"id":"acme","conservation":{"horizon_s":10,"trousseau":"/n/existe/pas.keys"}}],
            "sessions":[{"token":"t","tenant":"acme","role":"auditor","subject":"a"}]}"#,
    )
    .expect("écrit");
    let brut = std::fs::read_to_string(&decl).unwrap();
    let dep = Deploiement::depuis_json(&brut).expect("bien formée");
    let mut c = Console::new();
    let e = appliquer(&dep, &mut c, RACINE, 0).expect_err("doit refuser");
    assert!(e[0].to_string().contains("inexploitable"), "{:?}", e[0].to_string());
    menage(&[&decl]);
}

/// Les cas présents dans les records **sans clé** sont comptés : c'est la forme silencieuse de la
/// perte, celle qu'un exploitant ne découvrirait qu'en ouvrant un dossier.
#[test]
fn m6_cable_les_cas_sans_cle_sont_comptes() {
    let (records, keys, decl) = decor("sanscle");
    // On retire la clé du récent : son contenu deviendra illisible, et il faut le DIRE.
    let contenu = std::fs::read_to_string(&keys).unwrap();
    let ampute: String =
        contenu.lines().filter(|l| !l.starts_with("DOSSIER-RECENT")).collect::<Vec<_>>().join("\n");
    std::fs::write(&keys, ampute + "\n").unwrap();

    let brut = std::fs::read_to_string(&decl).unwrap();
    let dep = Deploiement::depuis_json(&brut).expect("bien formée");
    let mut c = Console::new();
    let resume = appliquer(&dep, &mut c, RACINE, 0).expect("saine");

    assert_eq!(resume.conservation_par_tenant["acme"], 1, "une seule clé chargée");
    assert_eq!(resume.cas_sans_cle["acme"], 1, "et un cas dont le contenu est perdu — annoncé");
    assert_eq!(lire(&mut c, "DOSSIER-RECENT"), Lecture::CleAbsente, "jamais « broyé » : on n'affirme pas une destruction qu'on n'a pas faite");
    menage(&[&records, &keys, &decl]);
}

// ═══ LE GARDE ═════════════════════════════════════════════════════════════════════════════════

/// **L'ordre des deux gestes EST la correction du module.** Le disque d'abord, la mémoire ensuite.
/// Les inverser donnerait, sur un échec d'écriture, un processus qui déclare détruit ce qui est
/// intact. Ce garde lit le code de `balayer` et exige cet ordre.
#[test]
fn m6_cable_le_disque_est_ecrit_avant_la_memoire() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/provisioning.rs"))
        .expect("provisioning.rs");
    let i = src.find("pub fn balayer(").expect("balayer");
    let corps = &src[i..];
    let fin = corps[1..].find("\n}\n").map_or(corps.len(), |j| j + 3);
    let code: String = corps[..fin]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    let disque = code.find("trousseau_fichier::broyer").expect("le disque doit être écrit");
    let memoire = code.find(".balayer(politique").expect("la mémoire doit être broyée");
    assert!(
        disque < memoire,
        "le trousseau doit être réécrit AVANT le broyage en mémoire : l'ordre inverse ferait, sur \
         un échec d'écriture, un processus qui déclare détruit ce qui est intact sur le disque"
    );
    // Et le broyage mémoire doit être DANS la branche de succès de l'écriture.
    let ok = code.find("Ok(_) =>").expect("branche de succès");
    assert!(ok < memoire && memoire < code.find("Err(e) =>").expect("branche d'échec"));
}

/// Le trousseau ne se déduit d'aucun secret : le fichier est la SEULE source des clés de cas.
/// Une clé dérivée se re-dériverait, et le broyage ne détruirait rien.
#[test]
fn m6_cable_aucune_cle_de_cas_nest_derivee_du_secret_racine() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/provisioning.rs"))
        .expect("provisioning.rs");
    let code: String =
        src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    assert!(!code.contains("CaseKey::from_bytes"), "aucune clé de cas ne se fabrique dans le câblage");
    assert_eq!(code.matches("TenantKey::derive").count(), 1, "seule la clé TENANT se dérive du root");
}

/// Un trousseau chargé rend le contenu lisible — la propriété que le redémarrage cassait avant ce
/// câblage, silencieusement, à chaque fois.
#[test]
fn m6_cable_sans_conservation_declaree_rien_nest_active() {
    let d = Deploiement::depuis_json(
        r#"{"tenants":[{"id":"acme"}],
            "sessions":[{"token":"t","tenant":"acme","role":"auditor","subject":"a"}]}"#,
    )
    .expect("bien formée");
    let mut c = Console::new();
    let r = appliquer(&d, &mut c, RACINE, 0).expect("saine");
    assert!(r.conservation_par_tenant.is_empty(), "aucune conservation déclarée, aucune activée");
    assert!(c.service_de("acme").unwrap().etat_conservation("X").is_none());
    assert!(balayer(&mut c, &d, 99_999).is_empty(), "et rien à balayer");
}
