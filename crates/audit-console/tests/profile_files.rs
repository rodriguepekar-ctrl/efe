// SPDX-License-Identifier: UNLICENSED
//! # T2 (console) — le profil livré dit-il ce que la console servira ?
//!
//! Jumeau de `efe-swarm-gateway/tests/profile_files.rs`, pour l'autre binaire. Il compare le
//! rapport produit à partir de `profiles/console.env` au `profiles/console.expected.json` livré
//! à côté.
//!
//! ## Pourquoi ce test existe, et pourquoi il ne touche pas l'environnement
//!
//! Un profil est une **promesse écrite** faite à l'opérateur : « déploie ceci, et voilà ce que tu
//! serviras ». Rien ne garantit qu'elle reste vraie — il suffit qu'un interrupteur change de classe
//! pour que le fichier mente sans que personne ne le remarque. Ce test fige la promesse.
//!
//! Il résout par [`Report::from_lookup`] plutôt que par `std::env::set_var` : deux tests qui
//! posent la même variable dans le même process se voleraient mutuellement leur réponse, et le
//! test qui échouerait serait choisi au hasard. La fonction est pure ; `from_env` n'en est qu'un
//! cas particulier, donc ce qui est vérifié ici est bien ce qui sera servi.

use audit_console::profile::{Class, Report, REGISTRY};
use std::collections::BTreeMap;

/// Le dossier `profiles/` à la racine du dépôt, atteint depuis le manifeste de ce crate.
fn profiles_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles")
}

/// Un `.env` réduit à ce dont un profil a besoin : `CLÉ=valeur`, `#` en commentaire.
fn parse_env(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

fn lu(profil: &str) -> BTreeMap<String, String> {
    let p = profiles_dir().join(format!("{profil}.env"));
    parse_env(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} illisible : {e}", p.display())))
}

/// Les profils de ce binaire. `console` = la posture de DÉMONSTRATION (un tenant, une session) ;
/// `service` = la posture de SERVICE que M7 rend possible (N tenants déclarés).
const PROFILS: &[&str] = &["console", "service"];

#[test]
fn chaque_profil_produit_exactement_le_rapport_quil_annonce() {
    for profil in PROFILS {
        let vars = lu(profil);
        let rapport = Report::from_lookup(|k| vars.get(k).cloned());

        let attendu: BTreeMap<String, String> = serde_json::from_str(
            &std::fs::read_to_string(profiles_dir().join(format!("{profil}.expected.json")))
                .unwrap_or_else(|e| panic!("{profil}.expected.json lisible : {e}")),
        )
        .unwrap_or_else(|e| panic!("{profil}.expected.json bien formé : {e}"));

        let obtenu: BTreeMap<String, String> =
            rapport.lines.iter().map(|l| (l.name.to_string(), l.status.to_string())).collect();

        assert_eq!(
            obtenu, attendu,
            "\nLe profil `{profil}` ne sert PAS ce que son `.expected.json` annonce.\n\
             Corriger le `.env` ou son attendu — mais ne jamais laisser les deux diverger : c'est\n\
             exactement le défaut que M0 existe pour supprimer.\n"
        );
    }
}

/// **M7.** En posture de service, la déclaration supplante quatre interrupteurs — et le rapport ne
/// crie plus que sur **UN** secret, pas deux.
///
/// C'est tout l'objet du statut « SANS OBJET » : sans lui, chaque démarrage d'un service annoncerait
/// « JETON PUBLIC — n'importe qui ouvre une session » à propos d'un jeton que ce service n'emploie
/// pas. Une fausse alarme au démarrage est la manière dont une vraie finit par être ignorée.
#[test]
fn le_profil_service_ne_crie_que_sur_le_secret_qui_reste_en_vigueur() {
    let vars = lu("service");
    let r = Report::from_lookup(|k| vars.get(k).cloned());

    assert_eq!(
        r.secrets_par_defaut,
        vec!["EFE_CONSOLE_ROOT"],
        "seul le secret RACINE reste en vigueur : la déclaration n'en porte aucun, les N clés en \
         sont toutes dérivées. Le jeton, lui, vient du fichier déclaré"
    );

    let sans_objet: Vec<&str> =
        r.lines.iter().filter(|l| l.status == "SANS OBJET").map(|l| l.name).collect();
    assert_eq!(
        sans_objet,
        audit_console::profile::SUPPLANTES_PAR_DECLARATION.to_vec(),
        "exactement les quatre interrupteurs de la posture de démonstration"
    );
}

/// Poser un interrupteur supplanté n'est pas neutre : l'opérateur croit qu'il agit. Le rapport doit
/// le lui dire, et lui dire de le retirer.
#[test]
fn un_interrupteur_supplante_mais_pose_est_signale_comme_ignore() {
    let mut vars = lu("service");
    vars.insert("EFE_CONSOLE_TOKEN".into(), "un-vrai-jeton".into());
    let r = Report::from_lookup(|k| vars.get(k).cloned());

    let l = r.lines.iter().find(|l| l.name == "EFE_CONSOLE_TOKEN").expect("déclaré");
    assert_eq!(l.status, "SANS OBJET", "posé ne veut pas dire servi");
    assert!(l.detail.contains("IGNORÉ"), "et on le DIT : {}", l.detail);
    assert!(l.detail.contains("Retirez-le"), "avec le geste à faire : {}", l.detail);
    assert!(!r.render_human().contains("un-vrai-jeton"), "sans jamais imprimer sa valeur");
}

/// La propriété centrale de ce profil : **il ne lui manque que les secrets**, et les injecter
/// suffit à éteindre l'avertissement. Si un jour il manquait autre chose, ce test tomberait.
#[test]
fn au_profil_console_il_ne_manque_que_les_deux_secrets() {
    let mut vars = lu("console");

    let avant = Report::from_lookup(|k| vars.get(k).cloned());
    assert_eq!(
        avant.secrets_par_defaut,
        vec!["EFE_CONSOLE_ROOT", "EFE_CONSOLE_TOKEN"],
        "sans injection, les deux secrets doivent être signalés — et EUX SEULS"
    );
    assert!(
        avant.render_human().contains("NE JAMAIS servir ainsi en production"),
        "la forme humaine doit CRIER : un défaut public silencieux est le pire des verts"
    );

    vars.insert("EFE_CONSOLE_ROOT".into(), "0".repeat(64));
    vars.insert("EFE_CONSOLE_TOKEN".into(), "1".repeat(64));

    let apres = Report::from_lookup(|k| vars.get(k).cloned());
    assert!(
        apres.secrets_par_defaut.is_empty(),
        "les secrets injectés, plus AUCUNE dégradation de classe secrète ne doit rester : {:?}",
        apres.secrets_par_defaut
    );
    assert!(
        !apres.render_human().contains("AVERTISSEMENT"),
        "l'avertissement doit DISPARAÎTRE une fois les secrets posés — sinon il devient du bruit, \
         et un avertissement permanent ne se lit plus"
    );
}

/// Le profil livré ne doit poser aucun secret : un secret versionné est un secret public, et il
/// ferait dire « SERVIE » à une ligne dont la valeur est connue de tous.
#[test]
fn aucun_profil_ne_pose_de_secret() {
    for profil in PROFILS {
        let vars = lu(profil);
        for s in REGISTRY.iter().filter(|s| s.class == Class::DegradedSecret) {
            assert!(
                !vars.contains_key(s.name),
                "{} est un secret : il ne doit JAMAIS être versionné dans un profil ({profil})",
                s.name
            );
        }
    }
}

/// `from_env` doit être un simple cas particulier de `from_lookup` — sinon le chemin testé et le
/// chemin servi peuvent diverger, et ce fichier ne prouverait plus rien du binaire.
#[test]
fn le_chemin_servi_et_le_chemin_teste_sont_le_meme() {
    let depuis_env = Report::from_env();
    let equivalent = Report::from_lookup(|n| std::env::var(n).ok());
    let a: Vec<_> = depuis_env.lines.iter().map(|l| (l.name, l.status)).collect();
    let b: Vec<_> = equivalent.lines.iter().map(|l| (l.name, l.status)).collect();
    assert_eq!(a, b, "`from_env` doit déléguer à `from_lookup`, pas dupliquer sa logique");
}
