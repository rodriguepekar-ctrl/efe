// SPDX-License-Identifier: UNLICENSED
//! **P3 — aucun nom de cible abandonnée dans le code des deux produits.**
//!
//! ## Ce que ce garde tient, et pourquoi il vaut mieux qu'un `grep`
//!
//! La feuille de route donnait pour critère `grep -c <nom>` → 0. Un `grep` mesure un **instant** ;
//! ce fichier tient un **état**. La différence n'est pas théorique dans ce dépôt : la feuille de
//! route elle-même recense trois occasions où un contrôle est devenu faux un module plus tard, et
//! le remède déjà trouvé par `m6-check.sh` était de ne pas ré-implémenter une version affaiblie du
//! garde mais de l'**exécuter**. `scripts/p3-check.sh` exécute donc celui-ci.
//!
//! ## Le mot n'est jamais écrit ici
//!
//! Il est reconstruit par [`concat!`]. Sans cette précaution le garde s'attraperait lui-même — le
//! piège que `un_seul_analyseur.rs` a rencontré le premier jour — et il faudrait alors l'exempter,
//! c'est-à-dire créer une exception dont personne ne se souviendrait de la raison.
//!
//! ## Il regarde les COMMENTAIRES, à l'inverse du garde d'autorité
//!
//! `un_seul_analyseur` saute les lignes de commentaire, et il a raison : une mention en commentaire
//! ne peut pas *diverger* d'un analyseur réel. Ici la règle s'inverse, parce que le grief s'inverse.
//! Les quatre citations que P3 nomme étaient **toutes** des commentaires, et le grief est de lecture,
//! pas de comportement : *« un prospect qui lit le code y verra le nom d'un autre »*. Un garde qui
//! sauterait les commentaires ne verrait donc exactement rien de ce que P3 demande de nettoyer.
//!
//! ## Ce qu'il ne regarde PAS : `docs/`
//!
//! Les documents d'architecture de juin sont des **archives datées d'un engagement réel**, et leur
//! prose nomme la partie en face — « ni la console, ni le binaire, ni … ». Réécrire ces phrases pour
//! satisfaire un critère cosmétique échangerait une petite gêne contre une falsification de
//! registre, ce qui est précisément le contraire de ce que ce dépôt vend. Seuls leurs **noms de
//! fichier** ont changé, parce que c'est le nom de fichier que le code cite.

use std::path::{Path, PathBuf};

/// Le nom proscrit, assemblé pour que ce fichier ne le contienne pas. Comparé en minuscules.
const MOT: &str = concat!("kov", "ant");

/// Les périmètres soustraits au garde, avec la raison — jamais une liste nue.
///
/// La discipline est celle de `un_seul_analyseur` : nommer, justifier, et ne jamais laisser la liste
/// grandir en silence. Chaque entrée est validée par [`refus_dexemption`], qui refuse un chemin
/// périmé et une justification trop courte pour dire quoi que ce soit.
const EXEMPTIONS: &[(&str, &str)] = &[
    // ── Deux exemptions RETIRÉES sur la branche c1-c3 ────────────────────────────────────────
    //
    // `crates/efe-meta-swarm` et `crates/efe-meta-swarm-server` figuraient ici avec, pour raison,
    // qu'ils n'appartiennent « à aucun des deux produits ». Sur cette branche, cette raison a cessé
    // d'être une exemption pour devenir un fait : les deux répertoires n'existent plus.
    //
    // Les retirer n'est pas contourner ce garde, c'est lui OBÉIR. `refus_dexemption` refuse
    // précisément une exemption dont la cible a disparu — « exemption périmée » — et il avait
    // raison de tomber. Une liste d'exemptions qui survit à ses cibles finit par exempter des
    // chemins que plus personne ne peut aller vérifier.
    (
        "crates/efe-meta-verify",
        "Même périmètre exclu. Son occurrence est l'identifiant d'une fixture qui alimente le HMAC \
         d'une chaîne méta, et le même identifiant est rejoué par le vérifieur croisé : le changer \
         d'un seul côté casserait la re-preuve croisée, et des deux côtés changerait des valeurs \
         scellées pour un gain nul.",
    ),
    (
        "crates/verify-meta-mini",
        "L'autre moitié du couple de re-preuve croisée décrit juste au-dessus. Les deux entrées \
         doivent rester identiques au caractère près, sans quoi le croisement ne croise plus rien.",
    ),
];

/// Tous les fichiers que ce garde regarde : le code et les manifestes, sous `crates/`.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entrees) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entrees.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs" || x == "toml") {
            out.push(p);
        }
    }
}

/// La racine du dépôt, atteinte depuis le manifeste de ce crate, **canonisée**.
///
/// Sans la canonisation le chemin garde ses `..`, et toute comparaison de préfixe échoue en
/// silence — un garde qui ne compare rien est un garde qui passe toujours.
fn racine() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("racine du dépôt")
}

/// **Le détecteur, isolé et pur** — pour qu'il soit éprouvable sur une entrée synthétique.
///
/// Un garde qui ne peut pas tomber ne garde rien. Le périmètre étant désormais propre, la boucle
/// principale ne trouve plus rien : elle serait donc verte même si ce détecteur était cassé — un
/// `contains` mal écrit suffirait. D'où l'extraction, et sa contre-épreuve sur un texte qui n'est
/// pas l'arbre du dépôt.
///
/// La comparaison est faite en minuscules : le nom apparaissait en trois casses différentes dans
/// le dépôt avant P3 (capitales dans deux commentaires de test, capitale initiale dans les renvois
/// de documentation, minuscules dans les identifiants).
fn fautes_dans(rel: &str, texte: &str) -> Vec<String> {
    let mut fautes = Vec::new();
    for (n, ligne) in texte.lines().enumerate() {
        if ligne.to_lowercase().contains(MOT) {
            fautes.push(format!("  {rel}:{}\n      {}", n + 1, ligne.trim()));
        }
    }
    fautes
}

/// **La validation d'une exemption, isolée et pure** — même raison que ci-dessus : la boucle qui
/// les parcourt ne peut pas prouver qu'elle sait refuser si aucune entrée n'est mauvaise.
fn refus_dexemption(prefixe: &str, pourquoi: &str, racine: &Path) -> Option<String> {
    if !racine.join(prefixe).is_dir() {
        return Some(format!(
            "exemption périmée : {prefixe} n'est plus un dossier — la retirer, ou corriger le chemin"
        ));
    }
    if pourquoi.len() <= 120 {
        return Some(format!(
            "une exemption doit dire POURQUOI le périmètre est hors des deux produits, pas seulement \
             qu'il l'est : {prefixe}"
        ));
    }
    None
}

/// Ce chemin relatif est-il couvert par une exemption ?
fn exempte(rel: &str) -> bool {
    EXEMPTIONS.iter().any(|(prefixe, _)| rel.starts_with(prefixe))
}

#[test]
fn aucun_nom_de_cible_abandonnee_dans_le_code_des_deux_produits() {
    let racine = racine();
    let mut fichiers = Vec::new();
    sources(&racine.join("crates"), &mut fichiers);
    fichiers.push(racine.join("Cargo.toml")); // le manifeste du workspace en portait une, lui aussi

    let mut fautes = Vec::new();
    let mut vus = 0usize;
    for f in &fichiers {
        let rel = f.strip_prefix(&racine).unwrap_or(f).to_string_lossy().replace('\\', "/");
        if exempte(&rel) {
            continue;
        }
        vus += 1;
        if let Ok(texte) = std::fs::read_to_string(f) {
            fautes.extend(fautes_dans(&rel, &texte));
        }
    }

    assert!(
        vus > 200,
        "le garde n'a regardé que {vus} fichiers — il ne balaye manifestement pas l'arbre attendu, \
         et un garde qui ne lit rien passe toujours"
    );
    assert!(
        fautes.is_empty(),
        "P3 : le nom d'une cible abandonnée le 11 août subsiste dans le code d'un des deux \
         produits.\nUn prospect qui lit ce code y verra le nom d'un autre.\n\n{}\n\n\
         Si le fichier appartient à un périmètre hors C1/C3, ajoutez-le aux EXEMPTIONS **avec sa \
         raison** — jamais en silence.",
        fautes.join("\n")
    );
}

#[test]
fn le_detecteur_sait_tomber() {
    // La contre-épreuve : sans elle, le test principal serait vert même détecteur cassé.
    let synthetique = format!("//! Cf. docs/reports/EFE_{MOT}_P1_Architecture.html\nfn x() {{}}\n");
    assert_eq!(
        fautes_dans("faux.rs", &synthetique).len(),
        1,
        "le détecteur doit voir une citation en commentaire — c'était la forme des QUATRE que P3 nomme"
    );

    // Les trois casses rencontrées dans le dépôt avant P3.
    for variante in [MOT.to_uppercase(), MOT.to_string(), {
        let mut c = MOT.chars();
        c.next().map(|p| p.to_uppercase().to_string() + c.as_str()).unwrap_or_default()
    }] {
        assert_eq!(
            fautes_dans("faux.rs", &format!("// {variante} produit l'artefact.")).len(),
            1,
            "casse « {variante} » manquée"
        );
    }

    // Et un texte propre ne doit rien produire, sinon le garde crierait sur tout.
    assert!(fautes_dans("propre.rs", "//! Un module tout à fait ordinaire.\n").is_empty());
}

#[test]
fn chaque_exemption_est_justifiee_et_vivante() {
    let racine = racine();
    let refus: Vec<String> =
        EXEMPTIONS.iter().filter_map(|(p, q)| refus_dexemption(p, q, &racine)).collect();
    assert!(refus.is_empty(), "{}", refus.join("\n"));
}

#[test]
fn le_validateur_dexemption_sait_refuser() {
    // Même raison que pour le détecteur : quatre exemptions valides ne prouvent pas qu'on sait en
    // refuser une mauvaise.
    let racine = racine();
    assert!(
        refus_dexemption("crates/ce-crate-nexiste-pas", &"x".repeat(200), &racine)
            .is_some_and(|m| m.contains("périmée")),
        "un chemin mort doit être refusé — sinon la liste survit à ce qu'elle exemptait"
    );
    assert!(
        refus_dexemption("crates/audit-console", "parce que.", &racine)
            .is_some_and(|m| m.contains("POURQUOI")),
        "une justification trop courte doit être refusée"
    );
    assert!(
        refus_dexemption("crates/audit-console", &"raison longue et circonstanciée. ".repeat(8), &racine)
            .is_none(),
        "une exemption valide doit passer"
    );
}
