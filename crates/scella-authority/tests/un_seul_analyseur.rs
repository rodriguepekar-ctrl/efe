// SPDX-License-Identifier: UNLICENSED
//! # Un seul analyseur d'autorité dans tout le dépôt — le garde qui rend M1 durable.
//!
//! ## Pourquoi le motif est le littéral `"://"`, et non un nom de fonction
//!
//! Le critère d'origine de la feuille de route était : *`grep -rln 'fn is_local_endpoint'` renvoie
//! un fichier*. Il compte un **nom**. La classe de vulnérabilité, elle, s'appelle **différentiel
//! d'analyseur** : elle naît dès que deux morceaux de code déduisent un hôte d'une même chaîne sans
//! obtenir le même résultat — ou sans obtenir celui que le client HTTP contactera. Ce qu'il faut
//! compter, ce sont donc les *analyseurs*.
//!
//! Trois des cinq analyseurs du dépôt ne portaient pas ce nom de fonction. Le critère pouvait donc
//! être satisfait — `grep` sortant 0 — avec le différentiel entier. Mesuré sur l'arbre, le littéral
//! `"://"` désigne exactement les analyseurs et rien d'autre ; `split_once("://")` en rate un, et
//! c'est justement celui qui portait le défaut vivant corrigé par P1.
//!
//! ## La dette est ici, datée et comptée — pas muette
//!
//! M1.a laisse deux analyseurs au méta-essaim, parce qu'ils décident autre chose (le SAN attendu en
//! mTLS, et si TLS peut être coupé) et que les migrer change un comportement observable en
//! production. Ils sont donc **exemptés explicitement**, avec la raison et la sortie prévue.
//!
//! Une liste d'exemptions est une vigilance déguisée en mécanisme — exactement ce que M1 combat.
//! Elle n'est tolérable qu'à trois conditions, tenues ici : elle est **courte**, chaque entrée dit
//! **pourquoi** et **quand** elle disparaît, et une assertion interdit qu'elle **grandisse**. Le
//! critère de fin de M1.b est qu'elle soit vide.

use std::path::{Path, PathBuf};

/// Les analyseurs que M1.a laisse en place, et la raison de chacun.
///
/// **M1.b doit ramener cette liste à zéro.** Elle ne doit jamais s'allonger : un nouvel analyseur
/// n'est pas une exception à documenter, c'est un bug à ne pas écrire.
const EXEMPTIONS: &[(&str, &str)] = &[
    (
        "efe-meta-swarm-server/src/controller/iterate.rs",
        "M1.b — extrait le SAN attendu de la vérification mTLS. Sûr aujourd'hui par composition \
         (une entrée malformée produit un SAN qu'aucun certificat ne porte, donc la connexion \
         échoue), pas par conception.",
    ),
    (
        "efe-meta-swarm-server/src/tls/mod.rs",
        "M1.b — décide si TLS peut être coupé sur un lien. Migrer change un comportement \
         observable : l'IPv6 nue `http://::1:8080` deviendra « non locale », donc TLS restera \
         allumé. C'est un durcissement, mais il exige sa propre note de version.",
    ),
];

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

/// La racine `crates/`, atteinte depuis le manifeste de ce crate.
///
/// Canonisée : sans cela le chemin garde son `..`, et toute comparaison de préfixe échoue en
/// silence — ce qui, la première fois, a fait que ce test s'est attrapé lui-même.
fn racine() -> PathBuf {
    let brut = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    std::fs::canonicalize(&brut).unwrap_or(brut)
}

#[test]
fn un_seul_analyseur_dautorite_dans_tout_le_depot() {
    let racine = racine();
    let mut fichiers = Vec::new();
    for e in std::fs::read_dir(&racine).expect("crates/ lisible").flatten() {
        let src = e.path().join("src");
        if src.is_dir() {
            sources(&src, &mut fichiers);
        }
    }
    assert!(
        fichiers.len() > 100,
        "seulement {} sources trouvées — le test se croirait vert à tort",
        fichiers.len()
    );

    let moi = concat!(env!("CARGO_PKG_NAME"), "/");
    let mut fautes = Vec::new();

    for f in &fichiers {
        let rel = f.strip_prefix(&racine).unwrap_or(f).display().to_string();
        // Ce crate EST l'analyseur : c'est le seul endroit du dépôt où le motif est légitime.
        // La comparaison porte sur le chemin RELATIF, pas sur un préfixe absolu — un chemin absolu
        // construit avec `..` ne se compare pas à un chemin canonisé.
        if rel.starts_with(moi) {
            continue;
        }
        if EXEMPTIONS.iter().any(|(chemin, _)| rel == *chemin) {
            continue;
        }
        let Ok(texte) = std::fs::read_to_string(f) else { continue };
        for (n, ligne) in texte.lines().enumerate() {
            // Les mentions en commentaire ou en documentation ne sont pas du code : elles ne
            // peuvent pas diverger de qui que ce soit.
            let nu = ligne.trim_start();
            if nu.starts_with("//") {
                continue;
            }
            if ligne.contains("\"://\"") {
                fautes.push(format!("  {rel}:{}\n      {}", n + 1, ligne.trim()));
            }
        }
    }

    assert!(
        fautes.is_empty(),
        "\nUn analyseur d'autorité est réapparu :\n{}\n\n\
         Utiliser `scella_authority::authority_of` / `host_of` / `is_local_endpoint`. Il est durci \n\
         contre les différentiels connus, et son accord avec le client HTTP réel est PROUVÉ par \n\
         `tests/accord_avec_url.rs`, pas espéré. Réécrire l'analyse est le défaut que M1 ferme : \n\
         les copies précédentes avaient perdu le durcissement en croyant rester autonomes.\n",
        fautes.join("\n")
    );
}

/// La liste d'exemptions ne doit pas grandir, et chaque entrée doit encore désigner un fichier réel
/// — une exemption périmée ne couvre plus rien et pourrait masquer un fichier renommé.
#[test]
fn la_liste_dexemptions_est_courte_datee_et_valide() {
    assert!(
        EXEMPTIONS.len() <= 2,
        "la liste d'exemptions a GRANDI ({} entrées). M1.b doit la vider, jamais l'étendre : \
         un nouvel analyseur n'est pas une exception à documenter, c'est un bug à ne pas écrire.",
        EXEMPTIONS.len()
    );
    for (chemin, pourquoi) in EXEMPTIONS {
        assert!(
            racine().join(chemin).is_file(),
            "exemption périmée : {chemin} n'existe plus — la retirer, ou corriger le chemin"
        );
        assert!(
            pourquoi.contains("M1.b"),
            "chaque exemption doit nommer le module qui la supprimera : {chemin}"
        );
        assert!(
            pourquoi.len() > 80,
            "une exemption doit dire POURQUOI, pas seulement qu'elle existe : {chemin}"
        );
    }
}
