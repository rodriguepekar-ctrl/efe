// SPDX-License-Identifier: UNLICENSED
//! # M9 — l'ancre tierce, éprouvée sur une vraie socket.
//!
//! Le critère : *« `AnchorProof` de méthode `rfc3161` **produite et vérifiée** »*. Avant ce module,
//! `AnchorProof` n'était construite que dans des tests, et `verify_anchor` rendait toujours
//! `crypto_receipt_checked: false`.
//!
//! Ce fichier fait tourner un horodateur **factice** sur une socket locale et va jusqu'au bout : la
//! demande part, le jeton revient, il est **lu**, sa liaison est **contrôlée**, la preuve est
//! déposée — et `forensic_export::verify_anchor` la déclare enfin `crypto_receipt_checked: true`.
//!
//! Et surtout le cas qui donne son sens au module : un horodateur qui rend un jeton engageant une
//! **autre** racine. Avant M9, on l'aurait déposé sans le lire.

use std::io::{Read, Write};
use std::net::TcpListener;

use forensic_export::anchor::{verify_anchor, AnchorMethod, AnchorOutcome, AnchorProof};
use provenance_bridge::anchor::{AnchorSink, PublishError};
use provenance_bridge::rfc3161::Rfc3161Sink;
use provenance_dag::AnchorRecord;
use scella_rfc3161::fabrique;

const RACINE_HEX: &str = "f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4f4";
const RACINE: [u8; 32] = [0xf4; 32];

/// Un horodateur factice : une connexion, une réponse, puis il s'arrête.
///
/// `engage` = le condensé que son jeton commettra. Le passer différent de ce qu'on soumet est ce qui
/// permet d'éprouver le contrôle de liaison — le cœur de M9.
fn tsa_factice(engage: [u8; 32], statut: u8, jeton: bool) -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("port libre");
    let port = l.local_addr().expect("adresse").port();
    std::thread::spawn(move || {
        let Ok((mut flux, _)) = l.accept() else { return };
        // Un vrai serveur lit la demande ENTIÈRE avant de répondre. Une première version répondait
        // après une seule lecture partielle et fermait : le client se prenait un RST en écrivant son
        // corps, et le test accusait le puits d'un défaut qui était celui du faux serveur.
        let mut recu: Vec<u8> = Vec::new();
        let mut morceau = [0u8; 4096];
        loop {
            let n = match flux.read(&mut morceau) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            recu.extend_from_slice(&morceau[..n]);
            let Some(sep) = recu.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
            let tete = String::from_utf8_lossy(&recu[..sep]).to_lowercase();
            let taille: usize = tete
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            if recu.len() >= sep + 4 + taille {
                break;
            }
        }
        let corps = if jeton {
            let j = fabrique::jeton_non_signe(&engage, "20260823120000Z");
            fabrique::reponse(statut, Some(&j))
        } else {
            fabrique::reponse(statut, None)
        };
        let entete = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/timestamp-reply\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            corps.len()
        );
        let _ = flux.write_all(entete.as_bytes());
        let _ = flux.write_all(&corps);
    });
    port
}

/// Un puits interne qui compte ce qu'on lui délègue — pour vérifier que la ligne de M4 survit.
struct JournalCompteur(std::sync::Arc<std::sync::atomic::AtomicUsize>);
impl Default for JournalCompteur {
    fn default() -> Self {
        JournalCompteur(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))
    }
}
impl AnchorSink for JournalCompteur {
    fn publish(&self, _a: &AnchorRecord) -> Result<(), PublishError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn describe(&self) -> String {
        "journal de test".into()
    }
}

fn repertoire(nom: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("scella-m9-{nom}-{}", std::process::id()));
    p
}

fn ancre() -> AnchorRecord {
    AnchorRecord::new("run-1", "acme", RACINE_HEX, 3, 1_700_000_000)
        .expect("ancre constructible")
}

// ═══ LE CRITÈRE ═══════════════════════════════════════════════════════════════════════════════

/// **Produite ET vérifiée.** La preuve sort du pont, et `verify_anchor` la déclare enfin
/// `crypto_receipt_checked: true` — ce qui était impossible avant M9.
#[test]
fn m9_une_preuve_rfc3161_est_produite_puis_verifiee() {
    let dir = repertoire("critere");
    let compteur = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = tsa_factice(RACINE, 0, true);

    let puits = Rfc3161Sink::nouveau(
        &format!("http://127.0.0.1:{port}/tsa"),
        &dir,
        Box::new(JournalCompteur(std::sync::Arc::clone(&compteur))),
    )
    .expect("URL décomposée par l'analyseur unique");

    puits.publish(&ancre()).expect("ancre obtenue et déposée");

    // Le jeton est sur le disque, TEL QUEL — c'est lui qu'un tiers passera à `openssl`.
    let jeton = std::fs::read(puits.chemin_jeton(RACINE_HEX)).expect("jeton déposé");
    assert!(!jeton.is_empty());

    // Et la preuve structurée se relit.
    let brut = std::fs::read(puits.chemin_preuve(RACINE_HEX)).expect("preuve déposée");
    let preuve = AnchorProof::decode(&brut).expect("preuve re-décodable");
    assert_eq!(preuve.method, AnchorMethod::Rfc3161);
    assert_eq!(preuve.bound_root, RACINE_HEX);
    assert_eq!(preuve.timestamp, 1_787_486_400, "l'instant vient du JETON, pas de notre horloge");
    assert_eq!(preuve.receipt, jeton);

    // ⭐ Le critère de M9.
    assert_eq!(
        verify_anchor(&preuve, RACINE_HEX),
        AnchorOutcome::Bound { crypto_receipt_checked: true },
        "avant M9, ce drapeau ne pouvait PAS être vrai : personne ne lisait le reçu"
    );

    // La ligne de journal de M4 survit — c'est elle qui détecte une TRONCATURE, et un horodatage ne
    // la remplace pas.
    assert_eq!(compteur.load(std::sync::atomic::Ordering::SeqCst), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// ⭐ **Le cas qui donne son sens au module.** L'horodateur rend un jeton engageant une AUTRE
/// racine. Avant M9, on l'aurait déposé sans le lire, et un auditeur aurait cru tenir un témoin.
#[test]
fn m9_un_jeton_qui_engage_une_autre_racine_nest_pas_depose() {
    let dir = repertoire("menteur");
    let compteur = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = tsa_factice([0x11; 32], 0, true);

    let puits = Rfc3161Sink::nouveau(
        &format!("http://127.0.0.1:{port}/tsa"),
        &dir,
        Box::new(JournalCompteur(std::sync::Arc::clone(&compteur))),
    )
    .expect("URL");

    let e = puits.publish(&ancre()).expect_err("doit refuser");
    let msg = e.to_string();
    assert!(msg.contains("N'ENGAGE PAS CETTE RACINE"), "{msg}");
    assert!(msg.contains("pire que pas de preuve"), "et pourquoi : {msg}");

    assert!(!puits.chemin_jeton(RACINE_HEX).exists(), "AUCUN fichier ne doit apparaître");
    assert!(!puits.chemin_preuve(RACINE_HEX).exists());
    assert_eq!(
        compteur.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "et le journal ne doit pas annoncer une ancre qui n'existe pas"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn m9_un_horodateur_qui_refuse_ne_produit_rien() {
    let dir = repertoire("refus");
    let port = tsa_factice(RACINE, 2, false);
    let puits = Rfc3161Sink::nouveau(
        &format!("http://127.0.0.1:{port}/"),
        &dir,
        Box::new(JournalCompteur::default()),
    )
    .expect("URL");
    let e = puits.publish(&ancre()).expect_err("refus");
    assert!(e.to_string().contains("statut PKI 2"), "{e}");
    assert!(!puits.chemin_jeton(RACINE_HEX).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Un statut accordé SANS jeton : on ne dépose rien plutôt que d'inventer une ancre.
#[test]
fn m9_un_accord_sans_jeton_ne_produit_rien() {
    let dir = repertoire("sansjeton");
    let port = tsa_factice(RACINE, 0, false);
    let puits = Rfc3161Sink::nouveau(
        &format!("http://127.0.0.1:{port}/"),
        &dir,
        Box::new(JournalCompteur::default()),
    )
    .expect("URL");
    let e = puits.publish(&ancre()).expect_err("sans jeton");
    assert!(e.to_string().contains("ancre inventée"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Un horodateur injoignable échoue **franchement**. C'est le prix annoncé de ce puits : il prouve
/// contre nous, et il dépend d'un tiers.
#[test]
fn m9_un_horodateur_injoignable_echoue_franchement() {
    let dir = repertoire("muet");
    let puits =
        Rfc3161Sink::nouveau("http://127.0.0.1:1/", &dir, Box::new(JournalCompteur::default()))
            .expect("URL");
    let e = puits.publish(&ancre()).expect_err("injoignable");
    assert!(e.to_string().contains("injoignable"), "{e}");
}

/// L'URL passe par l'analyseur UNIQUE du dépôt : ce qu'il refuse, ce puits le refuse.
#[test]
fn m9_lurl_est_decomposee_par_lanalyseur_unique() {
    for mauvaise in ["http://127.evil.com\\@x/", "http://héberge.fr/", "http://h:0/", "http://"] {
        assert!(
            Rfc3161Sink::nouveau(mauvaise, "/tmp", Box::new(JournalCompteur::default())).is_err(),
            "doit refuser ce que scella-authority refuse : {mauvaise}"
        );
    }
    let p = Rfc3161Sink::nouveau("http://exemple.fr/tsa", "/tmp", Box::new(JournalCompteur::default()))
        .expect("URL valide");
    assert!(p.describe().contains("exemple.fr"));
    assert!(p.describe().contains("openssl ts -verify"), "le describe DIT ce qui reste à faire");
}
