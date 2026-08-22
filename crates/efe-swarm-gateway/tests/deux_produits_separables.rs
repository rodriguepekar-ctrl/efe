// SPDX-License-Identifier: UNLICENSED
//! # M5 — joindre n'est pas souder : la séparation est mécanique, pas déclarative.
//!
//! ## Ce que ce test protège
//!
//! Ce dépôt porte deux produits qui se vendent séparément. Le **coin** — la passerelle souveraine
//! à reçu — exige de l'acheteur qu'il route son trafic par elle : un engagement d'architecture.
//! L'**expansion** — le registre d'évidence réglementaire — ne demande que des points
//! d'observation. Deux acheteurs, deux rythmes, deux ventes.
//!
//! L'analyse « produit unique » avait explicitement conclu de **ne pas les fusionner**. M5 les
//! joint quand même — c'est son objet — et toute la question est de joindre sans souder.
//!
//! ## Pourquoi une déclaration ne suffit pas
//!
//! « `reg-evidence` est optionnel » écrit dans un commentaire tient jusqu'au jour où quelqu'un
//! retire `optional = true` pour faire compiler quelque chose, ou ajoute la feature au défaut « le
//! temps d'un test ». Rien ne casse alors : le binaire du coin embarque simplement le registre,
//! et personne ne s'en aperçoit avant de compter les crates d'une fermeture de production.
//!
//! Ce test lit le manifeste et exige les trois conditions qui rendent la séparation vraie. La
//! quatrième — qu'aucun code non gardé ne référence `reg_evidence` — est déjà tenue par le
//! compilateur : le build sans feature échouerait.

use std::path::Path;

fn manifeste() -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} illisible : {e}", p.display()))
}

#[test]
fn le_coin_ne_transporte_pas_lexpansion_par_defaut() {
    let m = manifeste();

    // 1. La dépendance est OPTIONNELLE. Sans cela elle entre dans la fermeture de tout le monde.
    let ligne = m
        .lines()
        .find(|l| l.trim_start().starts_with("reg-evidence = {"))
        .expect("la dépendance `reg-evidence` doit être déclarée");
    assert!(
        ligne.contains("optional = true"),
        "\n`reg-evidence` n'est plus optionnelle :\n  {ligne}\n\n\
         Le binaire du coin embarquerait alors le registre — les deux produits ne seraient plus\n\
         séparables, et l'engagement d'architecture demandé à l'acheteur changerait sans que\n\
         personne ne l'ait décidé.\n"
    );

    // 2. Le DÉFAUT ne l'active pas. C'est la condition qu'un « juste pour voir » fait sauter.
    let defaut = m
        .lines()
        .find(|l| l.trim_start().starts_with("default = ["))
        .expect("la feature `default` doit être déclarée");
    assert!(
        !defaut.contains("reg-evidence"),
        "\nla feature par défaut active le registre :\n  {defaut}\n\n\
         Le coin ne doit RIEN embarquer de l'expansion tant que personne ne l'a demandé.\n"
    );

    // 3. La feature passe bien par `dep:` — sans quoi elle n'activerait pas la dépendance
    //    optionnelle et l'activation serait un mensonge silencieux : la feature « on », le code
    //    absent, et un rapport de démarrage qui annonce un pont qui n'existe pas.
    assert!(
        m.contains("reg-evidence = [\"dep:reg-evidence\"]"),
        "la feature `reg-evidence` doit activer la dépendance optionnelle via `dep:`"
    );
}

/// La réciproque, et elle compte autant : la JONCTION doit rester possible. Un test qui interdirait
/// le pont sans vérifier qu'il peut exister protégerait une séparation devenue une impasse.
#[test]
fn mais_la_jonction_reste_possible() {
    let m = manifeste();
    assert!(
        m.contains("reg-evidence = [\"dep:reg-evidence\"]"),
        "la feature qui JOINT les deux produits doit exister — sinon M5 n'a pas eu lieu"
    );
}

/// Le rapport de démarrage doit pouvoir dire que le pont n'est pas câblé, dans les DEUX builds.
/// Sans cela, un exploitant du build sans feature ne saurait pas pourquoi aucun dossier n'apparaît.
#[test]
fn le_rapport_nomme_le_pont_meme_sans_la_feature() {
    let noms: Vec<&str> = efe_swarm_gateway::profile::REGISTRY.iter().map(|s| s.name).collect();
    assert!(
        noms.contains(&"EFE_SWARM_REG_EVIDENCE_DIR"),
        "l'interrupteur du pont doit être au registre dans tous les builds — un opérateur qui ne \
         voit aucun dossier doit pouvoir lire POURQUOI"
    );
    assert!(noms.contains(&"EFE_SWARM_REG_FRAMEWORK"));
}
