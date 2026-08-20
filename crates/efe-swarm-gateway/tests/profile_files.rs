// SPDX-License-Identifier: UNLICENSED
//! **T2 (M0)** — *chaque profil produit exactement le rapport qu'il annonce*.
//!
//! Un profil qui dérive de son attendu est pire qu'un profil absent : l'opérateur croit savoir ce
//! qu'il déploie. Ce test compare, pour chacun des trois `.env`, la table
//! `capacité → état` au `.expected.json` livré à côté.
//!
//! ## Aucune mutation de l'environnement
//!
//! Le test **ne pose aucune variable dans le process** : il parse le fichier et passe une
//! fermeture à [`Resolved::from_lookup`], qui utilise **les mêmes fonctions pures** que le chemin
//! servi (`flag_of`, `present_of`). Deux tests qui muteraient l'environnement global se
//! courraient l'un l'autre — le dépôt évite déjà cela partout.

use std::collections::BTreeMap;
use std::path::PathBuf;

use efe_swarm_gateway::profile::{Report, Resolved};

fn profiles_dir() -> PathBuf {
    // `CARGO_MANIFEST_DIR` = crates/efe-swarm-gateway ; les profils vivent à la racine du dépôt.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join("profiles")
}

/// Parseur `.env` minimal et **délibérément strict** : `CLÉ=valeur`, `#` en commentaire, pas
/// d'interpolation. Un profil qui aurait besoin de plus serait un profil trop compliqué.
fn parse_env(text: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            m.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    m
}

fn statuses_for(profile: &str) -> BTreeMap<String, String> {
    let dir = profiles_dir();
    let text = std::fs::read_to_string(dir.join(format!("{profile}.env")))
        .unwrap_or_else(|e| panic!("profils/{profile}.env illisible : {e}"));
    let vars = parse_env(&text);
    let resolved = Resolved::from_lookup(|k| vars.get(k).cloned());
    Report::from_resolved(&resolved)
        .capabilities
        .iter()
        .map(|c| (c.capability.to_string(), c.status.as_str().to_string()))
        .collect()
}

fn expected_for(profile: &str) -> BTreeMap<String, String> {
    let dir = profiles_dir();
    let text = std::fs::read_to_string(dir.join(format!("{profile}.expected.json")))
        .unwrap_or_else(|e| panic!("profils/{profile}.expected.json illisible : {e}"));
    serde_json::from_str(&text).expect("attendu JSON invalide")
}

fn check(profile: &str) {
    let got = statuses_for(profile);
    let want = expected_for(profile);
    if got != want {
        let mut diff = String::new();
        for (k, w) in &want {
            match got.get(k) {
                Some(g) if g == w => {}
                Some(g) => diff.push_str(&format!("  {k:<22} attendu {w:<10} obtenu {g}\n")),
                None => diff.push_str(&format!("  {k:<22} attendu {w:<10} ABSENT du rapport\n")),
            }
        }
        for k in got.keys() {
            if !want.contains_key(k) {
                diff.push_str(&format!("  {k:<22} EN TROP dans le rapport\n"));
            }
        }
        panic!(
            "\nle profil « {profile} » ne produit pas le rapport qu'il annonce :\n{diff}\n\
             Corriger le `.env` ou son `.expected.json` — mais ne pas laisser les deux diverger.\n"
        );
    }
}

#[test]
fn t2_profil_demo() {
    check("demo");
}

#[test]
fn t2_profil_souverain() {
    check("souverain");
}

#[test]
fn t2_profil_frontiere() {
    check("frontiere");
}

/// **Le test qui prouve que les profils sont complets — secret excepté.**
///
/// `souverain` ne pose délibérément AUCUN secret : le rapport y annonce donc
/// `provenance : DÉGRADÉE`. Injecter le master — ce que fait un déploiement réel — doit suffire à
/// le faire passer `SERVIE`. Si ce test échoue, c'est qu'il manque autre chose que le secret, et
/// le profil est incomplet.
#[test]
fn le_profil_souverain_ne_lui_manque_que_le_secret() {
    let dir = profiles_dir();
    let text = std::fs::read_to_string(dir.join("souverain.env")).expect("souverain.env");
    let mut vars = parse_env(&text);

    let before = Report::from_resolved(&Resolved::from_lookup(|k| vars.get(k).cloned()));
    assert_eq!(
        before.status_of("provenance").map(|s| s.as_str()),
        Some("DÉGRADÉE"),
        "sans secret, le profil doit annoncer une provenance dégradée — jamais un faux vert"
    );

    vars.insert("EFE_NANO_AUDIT_HMAC_MASTER".into(), "0".repeat(64));
    let after = Report::from_resolved(&Resolved::from_lookup(|k| vars.get(k).cloned()));
    assert_eq!(
        after.status_of("provenance").map(|s| s.as_str()),
        Some("SERVIE"),
        "le seul manque du profil `souverain` doit être le secret"
    );
}

/// Un profil ne doit **jamais** contenir un secret. Le test le fige plutôt que de compter sur la
/// vigilance : c'est exactement la classe de faute qui ne se voit pas en relecture.
#[test]
fn aucun_profil_ne_contient_de_secret() {
    const INTERDITS: &[&str] = &[
        "EFE_NANO_AUDIT_HMAC_MASTER",
        "EFE_SWARM_BUNDLE_SIGNING_SEED",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GOOGLE_AI_KEY",
        "EFE_OIDC_PUBLIC_KEY_PEM",
        "DATABASE_URL",
    ];
    for profile in ["demo", "souverain", "frontiere"] {
        let text = std::fs::read_to_string(profiles_dir().join(format!("{profile}.env")))
            .unwrap_or_else(|e| panic!("{profile}.env : {e}"));
        let vars = parse_env(&text);
        for k in INTERDITS {
            assert!(
                !vars.contains_key(*k),
                "le profil « {profile} » POSE {k} — un secret dans un profil ferait dire au \
                 rapport « SERVIE » avec une valeur publiquement connue. Injecter au déploiement."
            );
        }
    }
}
