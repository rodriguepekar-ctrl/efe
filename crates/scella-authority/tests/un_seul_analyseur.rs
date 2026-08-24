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
//! ## La liste d'exemptions est VIDE depuis M1.b — et ce n'était pas une formalité
//!
//! M1.a laissait deux analyseurs au méta-essaim, exemptés nommément, au motif qu'ils décidaient
//! autre chose et qu'ils étaient « sûrs aujourd'hui par composition ». **Ce motif était faux pour
//! l'un des deux.** `efe-meta-swarm-server/src/tls/mod.rs` décidait si TLS pouvait être retiré, et
//! son `rsplit_once(':')` coupait au **dernier** deux-points :
//!
//! ```text
//! http://127.0.0.1:8080@evil.com/     ancienne lecture : hôte = 127.0.0.1  ⟹ « loopback »
//!                                     tonic::Endpoint  : hôte = evil.com
//! ```
//!
//! `IpAddr::from_str` recevait donc un `127.0.0.1` propre — la « composition » invoquée pour
//! rassurer était exactement ce qui faisait échouer le garde — et `connect_with_tls` retirait TLS.
//! Du gRPC en clair vers un hôte distant, annoncé en `debug!`. Cinq formes mesurées contre le vrai
//! client, cinq contournements. M1.b les a supprimées : le premier site est un **ré-export**, le
//! second est **supprimé** (son site d'appel interroge l'analyseur directement).
//!
//! ⭐ **La leçon à ne pas perdre** : une exemption documentée reste une exemption. Sa raison
//! écrite — ici « sûr par composition » — n'avait jamais été *mesurée*. Une dette qui se justifie
//! par un raisonnement non éprouvé n'est pas une dette, c'est un défaut qui a l'air d'un choix.
//!
//! ## Ce que ce garde couvre, et pourquoi c'est plus large qu'en M1.a
//!
//! M1.a ne lisait que `crates/*/src`. Un analyseur peut tout aussi bien vivre dans un harnais de
//! test, un banc ou un exemple — et c'est précisément là qu'une **copie de travail** se dépose
//! pendant qu'on migre. Le balayage couvre désormais tout `.rs` de chaque crate (hors `target/`).
//! Mesuré avant de l'élargir : l'arbre était déjà propre, donc l'élargissement ne coûte rien et
//! ferme un endroit réel.
//!
//! ## ⚠ La limite de ce garde, déclarée plutôt que découverte
//!
//! Un contrôle **textuel** attrape les copies **accidentelles** — c'est le vrai mode de défaillance
//! ici : cinq copies étaient nées sans que personne ne le décide. Il ne résiste pas à une
//! composition de chaîne : `concat!(':', '/', '/')` ne porte pas le motif et produit le même
//! séparateur.
//!
//! Le dépôt contient **exactement une** reproduction délibérée de l'ancien découpage, écrite ainsi,
//! et il faut la nommer plutôt que la laisser passer sans bruit :
//! `efe-meta-swarm-server/tests/m1b_pas_de_degradation_silencieuse.rs`, fonction
//! `ancien_est_loopback`. Son rôle est d'**échouer** : elle sert de contre-épreuve à l'invariant de
//! M1.b (« l'ancien analyseur viole-t-il bien ce que le nouveau tient ? »). Sans une telle copie,
//! on ne saurait pas si l'invariant a des dents.
//!
//! Ce n'est donc **pas** une exemption — rien n'est soustrait au garde, et la fonction n'analyse
//! rien pour le compte du dépôt. C'est une limite du mécanisme, et la déclarer ici est la seule
//! chose qui empêche qu'on la redécouvre un jour en croyant avoir trouvé une faille.

use std::path::{Path, PathBuf};

/// Les analyseurs exemptés du garde. **VIDE depuis M1.b.**
///
/// Le type reste en place, avec sa validation, pour une raison : si une exemption devait un jour
/// redevenir nécessaire, la discipline qui l'encadrait (nommer le module qui la supprime, dire
/// POURQUOI, ne jamais laisser la liste grandir en silence) doit s'appliquer immédiatement plutôt
/// qu'être réinventée. Un mécanisme conservé vide coûte trois lignes ; un mécanisme supprimé se
/// remplace par une vigilance.
const EXEMPTIONS: &[(&str, &str)] = &[];

/// Tous les `.rs` sous `dir`, `target/` exclu.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            // Un `target/` local à un crate contiendrait des sources générées et vendorées : les
            // lire ferait crier le garde sur du code que personne dans ce dépôt n'a écrit.
            if p.file_name().is_some_and(|n| n == "target") {
                continue;
            }
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

/// **Le détecteur, isolé et pur** — pour qu'il soit éprouvable sur une entrée synthétique.
///
/// Un garde qui ne peut pas tomber ne garde rien. Tant que la liste d'exemptions comptait des
/// entrées, le test principal *pouvait* tomber : il suffisait qu'un fichier non exempté porte le
/// motif. La liste étant vide, le test principal n'échoue plus jamais **si le détecteur est
/// cassé** — un `contains` mal écrit rendrait un vert éternel. D'où l'extraction : le détecteur a
/// sa propre contre-épreuve, sur un texte qui n'est pas l'arbre.
fn fautes_dans(rel: &str, texte: &str) -> Vec<String> {
    let mut fautes = Vec::new();
    for (n, ligne) in texte.lines().enumerate() {
        // Les mentions en commentaire ou en documentation ne sont pas du code : elles ne peuvent
        // pas diverger de qui que ce soit. (Ce fichier-ci en contient plusieurs, et le module de
        // `tls/mod.rs` en cite un pour expliquer le garde.)
        let nu = ligne.trim_start();
        if nu.starts_with("//") {
            continue;
        }
        if ligne.contains("\"://\"") {
            fautes.push(format!("  {rel}:{}\n      {}", n + 1, ligne.trim()));
        }
    }
    fautes
}

/// **La validation d'une exemption, isolée et pure** — même raison que ci-dessus : avec une liste
/// vide, la boucle du test serait vide, donc verte sans rien vérifier.
fn refus_dexemption(chemin: &str, pourquoi: &str, racine: &Path) -> Option<String> {
    if !racine.join(chemin).is_file() {
        return Some(format!("exemption périmée : {chemin} n'existe plus — la retirer, ou corriger le chemin"));
    }
    if !pourquoi.contains("M1.") {
        return Some(format!("chaque exemption doit nommer le module qui la supprimera : {chemin}"));
    }
    if pourquoi.len() <= 80 {
        return Some(format!("une exemption doit dire POURQUOI, pas seulement qu'elle existe : {chemin}"));
    }
    None
}

#[test]
fn un_seul_analyseur_dautorite_dans_tout_le_depot() {
    let racine = racine();
    let mut fichiers = Vec::new();
    for e in std::fs::read_dir(&racine).expect("crates/ lisible").flatten() {
        if e.path().is_dir() {
            sources(&e.path(), &mut fichiers);
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
        fautes.extend(fautes_dans(&rel, &texte));
    }

    assert!(
        fautes.is_empty(),
        "\nUn analyseur d'autorité est réapparu :\n{}\n\n\
         Utiliser `scella_authority::authority_of` / `host_of` / `is_local_endpoint`. Il est durci \n\
         contre les différentiels connus, et son accord avec le client HTTP réel est PROUVÉ par \n\
         `tests/accord_avec_url.rs`, pas espéré. Réécrire l'analyse est le défaut que M1 ferme : \n\
         les copies précédentes avaient perdu le durcissement en croyant rester autonomes, et \n\
         l'une d'elles retirait TLS vers un hôte distant (cf. M1.b).\n",
        fautes.join("\n")
    );
}

/// **La DoD de M1.b** : la liste est vide.
#[test]
fn la_liste_dexemptions_est_vide() {
    assert!(
        EXEMPTIONS.is_empty(),
        "la liste d'exemptions n'est pas vide ({} entrées) alors que M1.b l'a vidée. Un nouvel \
         analyseur n'est pas une exception à documenter, c'est un bug à ne pas écrire : la raison \
         écrite de l'exemption précédente (« sûr par composition ») s'est révélée FAUSSE à la \
         mesure. Si une exemption est vraiment inévitable, la mesurer d'abord — \
         `CORPUS_HOTE` et `CORPUS_LOCALITE` sont exportés pour cela.",
        EXEMPTIONS.len()
    );
    // Si une entrée revenait un jour, sa discipline s'applique dès la première seconde.
    let racine = racine();
    for (chemin, pourquoi) in EXEMPTIONS {
        assert!(refus_dexemption(chemin, pourquoi, &racine).is_none(), "{chemin}");
    }
}

// ─── Les contre-épreuves : les deux fonctions pures TOMBENT-ELLES ? ─────────────────────────
//
// Sans elles, ce fichier serait vert par vacuité — c'est le défaut que M7 avait laissé
// (`seules_les_deux_sondes_sont_publiques` ne pouvait pas tomber, et M8 a ajouté trois routes
// publiques sans le réveiller).

#[test]
fn le_detecteur_attrape_un_analyseur_reintroduit() {
    let faux = "fn h(e: &str) -> &str {\n    e.split_once(\"://\").map_or(e, |(_, r)| r)\n}\n";
    let f = fautes_dans("un/crate/src/lib.rs", faux);
    assert_eq!(f.len(), 1, "le détecteur n'a pas vu le motif : {f:?}");
    assert!(f[0].contains("un/crate/src/lib.rs:2"), "et il doit dire OÙ : {}", f[0]);
}

#[test]
fn le_detecteur_ignore_les_commentaires_mais_pas_le_code_qui_suit() {
    // Un commentaire qui CITE le motif — ce fichier et `tls/mod.rs` en contiennent.
    assert!(fautes_dans("x.rs", "// on cherche le littéral \"://\" dans le code\n").is_empty());
    assert!(fautes_dans("x.rs", "    /// le garde `\"://\"` ne verrait qu'un appel\n").is_empty());
    assert!(fautes_dans("x.rs", "//! le motif est \"://\"\n").is_empty());
    // Mais un commentaire de fin de ligne ne dispense pas la ligne.
    assert_eq!(fautes_dans("x.rs", "let a = s.split(\"://\"); // hop\n").len(), 1);
    // Et une URL entière n'est PAS le motif : c'est ce qui rend le grep utilisable sur des tests
    // qui manipulent des charges hostiles.
    assert!(fautes_dans("x.rs", "let e = \"http://127.0.0.1:8080@evil.com\";\n").is_empty());
}

#[test]
fn la_validation_dexemption_tombe_sur_chacun_de_ses_trois_motifs() {
    let racine = racine();
    let reel = "scella-authority/src/lib.rs"; // existe, relatif à `crates/`
    let bon = "M1.b — une raison suffisamment longue pour dire vraiment pourquoi cette exemption \
               existe et quand elle disparaît.";
    assert!(refus_dexemption(reel, bon, &racine).is_none(), "l'entrée valide doit passer");
    assert!(
        refus_dexemption("nexiste/pas.rs", bon, &racine).is_some_and(|m| m.contains("périmée")),
        "un chemin mort doit être refusé"
    );
    assert!(
        refus_dexemption(reel, &bon.replace("M1.b", "plus tard"), &racine)
            .is_some_and(|m| m.contains("nommer le module")),
        "une exemption sans module de sortie doit être refusée"
    );
    assert!(
        refus_dexemption(reel, "M1.b — trop court", &racine).is_some_and(|m| m.contains("POURQUOI")),
        "une exemption sans raison doit être refusée"
    );
}
