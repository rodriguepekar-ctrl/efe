// SPDX-License-Identifier: UNLICENSED
//! **L'endpoint de sortie est configurable — et le reçu dit où les données sont parties.**
//!
//! ## Le grief
//!
//! L'URL de sortie du provider frontière était une **constante compilée**. Trois conséquences, dont
//! la troisième est celle qui a rendu ce fichier nécessaire :
//!
//! 1. impossible de pointer un **faux fournisseur local**, donc l'aller-retour frontière n'a jamais
//!    été joué — c'est la première ligne de ce que `scripts/p1-check.sh` déclare ne pas prouver ;
//! 2. impossible de pointer un **mandataire d'entreprise**, un Azure OpenAI, ou un point de
//!    terminaison régional : une passerelle qui se dit souveraine et dont la destination est gravée
//!    vers les États-Unis ne l'est qu'à moitié ;
//! 3. et si l'on rendait l'URL configurable **en oubliant `endpoint()`**, le reçu scellé
//!    continuerait d'annoncer `api.anthropic.com` pendant que les données partiraient ailleurs.
//!    Ce serait un **faux** dans un artefact conçu pour être opposable. C'est le risque que ce
//!    fichier existe pour tenir.
//!
//! ## Ce qu'il ne prouve pas
//!
//! Rien ici n'ouvre de socket. Ces tests vérifient la **déclaration** de l'endpoint — ce que le
//! provider dit de sa destination — et son refus des formes dangereuses. Que l'octet parte bien à
//! cette adresse est l'affaire de `scripts/p1-check.sh`, qui lance un vrai faux fournisseur.

// `ClaudeProvider` vit sous la feature `anthropic`, qui n'est PAS dans le défaut de ce crate
// (`default = []`). Sans cette ligne le fichier ne compilerait pas sous `cargo test --workspace` ;
// avec elle il est simplement vide — ce qui serait pire si personne ne le jouait jamais. C'est
// `scripts/p1-check.sh` qui le joue, sous `--features anthropic`, et qui ÉCHOUE si zéro test a
// tourné. Le précédent est sa propre section 6 : « le critère de M3, que personne ne jouait ».
#![cfg(feature = "anthropic")]

use efe_coordinator::llm::providers::auth::ApiKey;
use efe_coordinator::{ClaudeProvider, LlmProvider};

const DEFAUT: &str = "https://api.anthropic.com/v1/messages";

fn provider() -> ClaudeProvider {
    ClaudeProvider::new(ApiKey::new("sk-ant-test-0123456789abcdef"), "claude-test")
}

#[test]
fn sans_rien_faire_la_destination_est_inchangee() {
    assert_eq!(
        provider().endpoint().as_deref(),
        Some(DEFAUT),
        "le défaut doit rester exactement ce qu'il était : rendre une URL configurable ne doit \
         changer la destination de personne qui n'a rien demandé"
    );
}

#[test]
fn le_recu_annonce_la_destination_reelle_pas_la_constante() {
    let p = provider()
        .with_endpoint("http://127.0.0.1:8099/v1/messages")
        .expect("un faux fournisseur local doit être accepté");
    assert_eq!(
        p.endpoint().as_deref(),
        Some("http://127.0.0.1:8099/v1/messages"),
        "C'EST LE TEST QUI COMPTE. `endpoint()` alimente `ProvenanceProvider`, qui scelle la \
         destination dans le reçu. S'il rendait la constante, le reçu nommerait Anthropic pendant \
         que les données partiraient ailleurs — un faux, dans l'artefact même qui doit être \
         opposable."
    );
}

#[test]
fn un_point_de_terminaison_regional_ou_un_mandataire_est_accepte() {
    for u in ["https://eu.exemple.test/v1/messages", "https://mandataire.interne:8443/v1/messages"] {
        let p = provider().with_endpoint(u).unwrap_or_else(|e| panic!("{u} refusé : {e}"));
        assert_eq!(p.endpoint().as_deref(), Some(u));
    }
}

#[test]
fn http_distant_est_refuse_et_la_destination_ne_bouge_pas() {
    let e = provider().with_endpoint("http://api.exemple.test/v1/messages").unwrap_err();
    assert!(format!("{e}").contains("machine locale"), "{e}");
    // Le refus ne doit rien laisser derrière : un provider neuf garde le défaut.
    assert_eq!(provider().endpoint().as_deref(), Some(DEFAUT));
}

/// La forme exacte qui a coûté 875 cas à M1.b. Elle ne doit pas passer par la porte loopback.
#[test]
fn la_ruse_du_userinfo_ne_franchit_pas_la_porte_locale() {
    let e = provider().with_endpoint("http://127.0.0.1:8099@evil.test/v1/messages").unwrap_err();
    let m = format!("{e}");
    assert!(m.contains("machine locale"), "{m}");
    assert!(!m.contains("evil.test/v1"), "la raison ne doit pas recopier l'URL brute : {m}");
}

#[test]
fn un_identifiant_dans_lurl_est_refuse_car_lendpoint_est_scelle() {
    let e = provider().with_endpoint("https://cle:secrete@api.exemple.test/v1/m").unwrap_err();
    let m = format!("{e}");
    assert!(m.contains("userinfo"), "{m}");
    assert!(!m.contains("secrete"), "le message d'erreur NE DOIT PAS recopier le secret : {m}");
}

/// Contre-épreuve du fichier : sans elle, un `with_endpoint` qui ignorerait son argument
/// passerait le premier test et le quatrième, et l'on ne verrait rien.
#[test]
fn contre_epreuve_with_endpoint_change_vraiment_quelque_chose() {
    let avant = provider().endpoint();
    let apres = provider().with_endpoint("https://autre.exemple.test/v1/m").unwrap().endpoint();
    assert_ne!(avant, apres, "with_endpoint doit modifier la destination déclarée");
}
