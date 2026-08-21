// SPDX-License-Identifier: UNLICENSED
//! # P1 — la passerelle n'a plus d'analyseur d'autorité, et n'en reprendra pas.
//!
//! ## Le trou que ce test ferme
//!
//! `frontier.rs` portait un analyseur d'hôte écrit sur place, justifié par « miroir minimal […]
//! **sans en dépendre** ». Ce miroir avait perdu le durcissement qu'il prétendait refléter : il
//! concluait au loopback sur `starts_with("127.")`, de sorte que `http://127.evil.com` — un nom DNS
//! ordinaire — passait pour local et n'émettait aucun avertissement.
//!
//! Et la dépendance qu'il évitait était **déjà payée** : ce fichier n'est compilé que sous la
//! feature `frontier-llm`, précisément celle qui active `provenance-capture`.
//!
//! ## Pourquoi le motif est le littéral `"://"`, et pas un nom de fonction
//!
//! Compter `fn is_local_endpoint` compte un *nom*. La classe de vulnérabilité s'appelle
//! **différentiel d'analyseur** : elle naît dès que deux morceaux de code déduisent un hôte d'une
//! même chaîne sans obtenir le même résultat — ou sans obtenir celui que le client HTTP contactera.
//! Ce qu'il faut compter, ce sont donc les *analyseurs*. Mesuré sur le dépôt, le littéral `"://"`
//! en désigne exactement cinq et rien d'autre ; `split_once("://")` n'en trouve que quatre et rate
//! justement celui qui portait le défaut.
//!
//! Ce test applique le critère à la passerelle seule — le périmètre que P1 nettoie. L'étendre au
//! dépôt entier est l'objet de M1, qui doit encore traiter les deux analyseurs du méta-essaim.

use std::path::{Path, PathBuf};

/// Aucune exemption. Si un jour il en faut une, c'est que le raisonnement ci-dessus a été perdu :
/// écrire *pourquoi*, datée, plutôt que d'ajouter une ligne muette.
const EXEMPTIONS: &[&str] = &[];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn la_passerelle_ne_contient_aucun_analyseur_dautorite() {
    let racine = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut fichiers = Vec::new();
    sources(&racine, &mut fichiers);
    assert!(!fichiers.is_empty(), "aucune source trouvée — le test se croirait vert à tort");

    let mut fautes = Vec::new();
    for f in &fichiers {
        let rel = f.strip_prefix(&racine).unwrap_or(f).display().to_string();
        if EXEMPTIONS.contains(&rel.as_str()) {
            continue;
        }
        let Ok(texte) = std::fs::read_to_string(f) else { continue };
        for (n, ligne) in texte.lines().enumerate() {
            // Le motif est le littéral Rust `"://"` — la marque d'un code qui coupe un schéma
            // d'URI pour en tirer une autorité. Les mentions en commentaire ou en documentation
            // (`// … "://" …`) ne sont pas du code et ne peuvent pas diverger.
            let nu = ligne.trim_start();
            if nu.starts_with("//") || nu.starts_with("///") || nu.starts_with("//!") {
                continue;
            }
            if ligne.contains("\"://\"") {
                fautes.push(format!("  {rel}:{}  {}", n + 1, ligne.trim()));
            }
        }
    }

    assert!(
        fautes.is_empty(),
        "\nUn analyseur d'autorité est réapparu dans la passerelle :\n{}\n\n\
         Utiliser `provenance_capture::host_of` / `is_local_endpoint` — durcis, testés contre les \n\
         différentiels SSRF, et déjà dans la portée sous `frontier-llm`. Réécrire l'analyse est le \n\
         défaut que P1 a fermé : la copie précédente avait perdu le durcissement en croyant \n\
         économiser une dépendance qui était déjà payée.\n",
        fautes.join("\n")
    );
}
