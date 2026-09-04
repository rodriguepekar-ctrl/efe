// SPDX-License-Identifier: UNLICENSED
//! **R3 — un SEUL fournisseur cryptographique dans le verrou.**
//!
//! ## Ce que R3 cherchait, et ce que R3 était vraiment
//!
//! R3 est né d'une observation : deux tests « passaient seuls et échouaient en masse ». On les a
//! d'abord appelés *instables*. La mesure du 2026-08-27 a réfuté ce mot. Ils étaient parfaitement
//! **déterministes** — 10 échecs sur 10 en rejouant le binaire de la course `--workspace` seul, au
//! repos. Ce n'était pas de l'instabilité : c'était une comparaison entre **deux binaires
//! différents**.
//!
//! Cargo unifie les features par caisse sur l'ensemble résolu. Sous `--workspace`,
//! `efe-agent-runner → axum-server "tls-rustls"` allumait un **SECOND** fournisseur
//! cryptographique rustls. Le binaire liait les deux — 19 symboles `aws-lc` et 54 `ring`, contre
//! 0 et 77 sous `-p` — et rustls **refuse de deviner** lequel employer : panique au démarrage du
//! processus, avant tout test. Sous `-p`, `efe-agent-runner` n'entrait pas dans la résolution, un
//! seul fournisseur était lié, et « le test passait ». Il ne passait pas : un AUTRE binaire passait.
//!
//! ## Pourquoi ce fichier existe maintenant
//!
//! **La cause a disparu — et personne ne l'a corrigée.** É0 a retiré 47 membres du workspace,
//! `efe-agent-runner` avec eux. `axum-server` et `aws-lc-rs` ont quitté `Cargo.lock` du même coup.
//! Le mode de défaillance n'est pas réparé : il est **hors de portée**, tant que le verrou ne
//! porte qu'un fournisseur.
//!
//! C'est une propriété fragile, et invisible : elle ne tient à aucune ligne de code. Une seule
//! dépendance ajoutée — un serveur HTTP avec TLS, un client qui préfère `aws-lc-rs` — la casse
//! sans qu'aucun test métier ne bronche, et la panique revient sous `--workspace` en ressemblant à
//! de l'instabilité. C'est-à-dire au verdict que R3 a déjà rendu, et qui était faux.
//!
//! Le remède que R3 prescrivait — `serial_test` — est **inopérant** ici, et sa propre note le dit :
//! il sérialise DANS un binaire, alors que la panique naît de la RÉSOLUTION des features, avant
//! qu'un test ne s'exécute. Le seul garde qui morde est celui-ci : surveiller le verrou.
//!
//! ## Ce que ce fichier ne prouve pas
//!
//! Que le fournisseur restant fonctionne, ni qu'un fournisseur unique suffise à tous les usages.
//! Il prouve exactement une chose : **le verrou n'en porte qu'un**, donc rustls n'a rien à deviner.

use std::path::{Path, PathBuf};

/// Les trois caisses qui, présentes ensemble, redonnent à rustls un choix à faire.
///
/// `ring` et `aws-lc-rs` sont les deux fournisseurs ; `aws-lc-sys` est le portage natif du second
/// et le trahit même si l'enrobage change de nom. La liste est courte **volontairement** : un
/// détecteur qui ratisse large attrape des homonymes et perd la confiance qu'on lui accorde.
const FOURNISSEURS: &[&str] = &["ring", "aws-lc-rs", "aws-lc-sys"];

/// Le plancher anti-vacuité. Un `Cargo.lock` illisible, tronqué, ou cherché au mauvais endroit rend
/// « zéro fournisseur » — un vert parfait sur une mesure qui n'a pas eu lieu. Le périmètre en
/// comptait 413 le jour où ce garde a été écrit ; 100 laisse toute la place à un élagage futur et
/// aucune à un fichier vide.
const PLANCHER_PAQUETS: usize = 100;

fn verrou() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock")
}

/// Le détecteur, **pur** — pour que sa contre-épreuve soit possible sans toucher au disque.
///
/// Rend `(noms des fournisseurs trouvés, nombre total de paquets)`. On lit le champ `name` en
/// début de ligne : c'est la forme que `cargo` écrit, et la seule qui distingue la DÉCLARATION
/// d'un paquet de sa mention dans la liste `dependencies` d'un autre.
fn fournisseurs_dans(verrou: &str) -> (Vec<String>, usize) {
    let mut trouves = Vec::new();
    let mut paquets = 0usize;
    for ligne in verrou.lines() {
        let Some(reste) = ligne.strip_prefix("name = \"") else { continue };
        let Some(nom) = reste.strip_suffix('"') else { continue };
        paquets += 1;
        if FOURNISSEURS.contains(&nom) && !trouves.iter().any(|d| d == nom) {
            trouves.push(nom.to_string());
        }
    }
    trouves.sort();
    (trouves, paquets)
}

/// ⭐ **Le garde.** Un fournisseur, pas deux.
#[test]
fn un_seul_fournisseur_cryptographique_dans_le_verrou() {
    let chemin = verrou();
    let texte = std::fs::read_to_string(&chemin)
        .unwrap_or_else(|e| panic!("Cargo.lock illisible en « {} » : {e}", chemin.display()));

    let (trouves, paquets) = fournisseurs_dans(&texte);

    assert!(
        paquets >= PLANCHER_PAQUETS,
        "seulement {paquets} paquet(s) lus dans « {} » — en dessous du plancher de \
         {PLANCHER_PAQUETS}. Ce n'est pas un verdict sur les fournisseurs, c'est l'aveu que le \
         verrou n'a pas été lu.",
        chemin.display()
    );

    assert_eq!(
        trouves.len(),
        1,
        "le verrou porte {} fournisseur(s) cryptographique(s) — {trouves:?} — sur {paquets} \
         paquets.\n\n\
         Deux fournisseurs liés dans un même binaire font PANIQUER rustls au démarrage du \n\
         processus, avant qu'un seul test ne s'exécute. Sous `cargo test --workspace` cela \n\
         ressemble à de l'instabilité : des tests « qui passent seuls » (autre binaire, autre \n\
         résolution de features) et « échouent en masse ». C'est le faux verdict que R3 a rendu \n\
         le 2026-08-27, et que ce garde existe pour empêcher de rendre deux fois.\n\n\
         Ce qui vient d'être ajouté au workspace tire un second fournisseur. Le remède n'est pas \n\
         `serial_test` — il sérialise DANS un binaire, et la panique naît AVANT lui, à la \n\
         résolution. C'est la dépendance qu'il faut choisir : désactiver ses features TLS par \n\
         défaut, ou aligner tout le périmètre sur un seul fournisseur.",
        trouves.len()
    );
}

// ─── Les contre-épreuves : ce détecteur sait-il seulement dire NON ? ────────────────────────
//
// Sans elles, ce fichier serait vert par vacuité — c'est la faute que M7 avait laissée
// (`seules_les_deux_sondes_sont_publiques` ne pouvait pas tomber) et que ce dépôt corrige partout.

#[test]
fn le_detecteur_voit_un_second_fournisseur() {
    let faux = concat!(
        "[[package]]\nname = \"ring\"\nversion = \"0.17.8\"\n\n",
        "[[package]]\nname = \"aws-lc-rs\"\nversion = \"1.8.1\"\n\n",
        "[[package]]\nname = \"serde\"\nversion = \"1.0\"\n",
    );
    let (trouves, paquets) = fournisseurs_dans(faux);
    assert_eq!(paquets, 3, "les trois paquets doivent être comptés");
    assert_eq!(trouves, vec!["aws-lc-rs".to_string(), "ring".to_string()], "les DEUX doivent être vus");
}

#[test]
fn le_detecteur_ne_confond_pas_une_dependance_avec_une_declaration() {
    // Un paquet qui DÉPEND de `ring` le mentionne dans sa liste `dependencies`. Le compter là
    // ferait voir des fournisseurs partout : `ring` a neuf dépendants directs dans ce verrou.
    let faux = concat!(
        "[[package]]\nname = \"rustls\"\nversion = \"0.23\"\ndependencies = [\n \"ring\",\n \"aws-lc-rs\",\n]\n",
    );
    let (trouves, paquets) = fournisseurs_dans(faux);
    assert_eq!(paquets, 1, "un seul paquet est DÉCLARÉ");
    assert!(trouves.is_empty(), "aucun fournisseur n'est déclaré ici, seulement mentionné : {trouves:?}");
}

#[test]
fn le_plancher_attrape_un_verrou_vide() {
    let (trouves, paquets) = fournisseurs_dans("");
    assert_eq!(paquets, 0);
    assert!(trouves.is_empty());
    assert!(
        paquets < PLANCHER_PAQUETS,
        "un verrou vide doit tomber SOUS le plancher — sinon le plancher ne garde rien"
    );
}
