// SPDX-License-Identifier: UNLICENSED
//! **M0 — le troisième état, porté à la console.**
//!
//! ## Le trou que ces gardes ferment
//!
//! M0 a livré, pour la passerelle, le cas que ni « présente » ni « absente » ne décrit : une valeur
//! **fournie mais refusée**, c'est-à-dire le seul cas où l'opérateur a agi sans que son geste ait le
//! moindre effet. La console n'en avait rien.
//!
//! Le résultat était mesurable. `EFE_CONSOLE_BALAYAGE_S` est un nombre de secondes, et il subissait
//! **deux substitutions silencieuses à deux endroits** :
//!
//! * `main.rs` faisait `read::value(...).parse().unwrap_or(3600)` — `1h`, `3600s` ou `quotidien`
//!   retombaient sur 3600 sans un mot ;
//! * `server.rs` faisait `.max(1)` — `0` devenait **un balayage toutes les secondes**, c'est-à-dire
//!   l'inverse de ce que `0` suggère à qui l'écrit.
//!
//! Et le rapport, lui, affichait `SERVIE` avec la chaîne brute. Un exploitant lisait donc
//! « `EFE_CONSOLE_BALAYAGE_S  SERVIE  1h` » pendant que le binaire balayait toutes les heures.
//! Le rapport contredisait le service : exactement ce que M0 existe pour empêcher.
//!
//! ## Pourquoi le critère n'est pas « la valeur est-elle acceptée ? »
//!
//! C'est la règle que la passerelle a fixée et qu'on reprend ici : **le résultat correspond-il à
//! l'intention ?** Une extinction délibérée n'est pas un footgun ; une valeur illisible qui retombe
//! par accident sur le défaut en est un. `3600` → 3600 : intention respectée. `1h` → 3600 : intention
//! trahie. `0` → 1 seconde : intention inversée.

use audit_console::profile::{
    entier_from_lookup, invalides_from_lookup, switch, Report, NUMERIQUES, REGISTRY,
};

/// Une table de valeurs — la seule forme testable sans muter l'environnement partagé du process.
fn table<'a>(paires: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |n: &str| paires.iter().find(|(k, _)| *k == n).map(|(_, v)| (*v).to_string())
}

fn ligne<'a>(r: &'a Report, nom: &str) -> &'a audit_console::profile::Line {
    r.lines.iter().find(|l| l.name == nom).expect("l'interrupteur est au registre")
}

const BAL: &str = "EFE_CONSOLE_BALAYAGE_S";

// ══════════════════════════════════════════════════════════════════════════════════════
//  Le comportement : les quatre cas, chacun distinct des trois autres
// ══════════════════════════════════════════════════════════════════════════════════════

#[test]
fn une_valeur_illisible_est_refusee_et_non_servie() {
    let r = Report::from_lookup(table(&[(BAL, "1h")]));
    let l = ligne(&r, BAL);
    assert_eq!(l.status, "REFUSÉE", "avant M0 cette ligne annonçait SERVIE : {}", l.detail);
    assert!(l.detail.contains("1h"), "le rapport doit citer ce que l'opérateur a écrit : {}", l.detail);
    assert!(l.detail.contains("3600"), "et ce que le binaire emploie réellement : {}", l.detail);
    assert_eq!(r.invalides.len(), 1);
}

#[test]
fn zero_est_refuse_parce_quil_donnait_exactement_linverse() {
    // `0` se PARSE parfaitement — c'est un entier. Le refuser n'est donc pas une affaire de syntaxe
    // mais de sens : `.max(1)` en faisait un balayage à la seconde, quand celui qui écrit `0` veut
    // presque toujours dire « désactivé ».
    let r = Report::from_lookup(table(&[(BAL, "0")]));
    assert_eq!(ligne(&r, BAL).status, "REFUSÉE");
    assert_eq!(r.invalides[0].effectif, 3600, "et l'on dit CE QUI TOURNE, pas seulement le refus");
}

#[test]
fn une_valeur_recevable_ne_declenche_rien() {
    let r = Report::from_lookup(table(&[(BAL, "7200")]));
    assert_eq!(ligne(&r, BAL).status, "SERVIE");
    assert!(r.invalides.is_empty(), "un garde qui crie sur du valide cesse d'être lu");
    assert_eq!(entier_from_lookup(BAL, table(&[(BAL, "7200")])), 7200);
}

#[test]
fn labsence_nest_pas_un_refus() {
    // Confondre les deux noierait le vrai signal : une absence est déjà rapportée en « DÉFAUT », et
    // c'est une situation, pas une faute.
    for vide in ["", "   "] {
        let r = Report::from_lookup(table(&[(BAL, vide)]));
        assert!(r.invalides.is_empty(), "« {vide} » est une absence, pas un footgun");
        assert_eq!(ligne(&r, BAL).status, "DÉFAUT");
    }
    let r = Report::from_lookup(|_: &str| None);
    assert!(r.invalides.is_empty());
    assert_eq!(ligne(&r, BAL).status, "DÉFAUT");
}

// ══════════════════════════════════════════════════════════════════════════════════════
//  T5 — le rapport et le service ne peuvent pas se contredire
// ══════════════════════════════════════════════════════════════════════════════════════

#[test]
fn ce_que_le_rapport_annonce_est_ce_que_le_binaire_emploie() {
    // La propriété, sur toutes les formes qu'un opérateur écrit réellement — y compris les bonnes.
    for v in ["3600", "7200", "1", "0", "1h", "3600s", "quotidien", "-5", "", "  42  "] {
        let paires = [(BAL, v)];
        let f = table(&paires);
        let effectif = entier_from_lookup(BAL, &f);
        let r = Report::from_lookup(&f);
        match r.invalides.iter().find(|i| i.name == BAL) {
            Some(inv) => assert_eq!(
                inv.effectif, effectif,
                "« {v} » : le rapport annonce {} et le service emploierait {effectif}",
                inv.effectif
            ),
            // Pas de refus : alors la valeur employée doit être celle que l'opérateur a posée, ou le
            // défaut du registre si rien n'était posé. Dans les deux cas, rien à taire.
            None => {
                let attendu: u64 = if v.trim().is_empty() {
                    switch(BAL).unwrap().default.parse().unwrap()
                } else {
                    v.trim().parse().expect("non refusée ⇒ analysable")
                };
                assert_eq!(effectif, attendu, "« {v} » : valeur servie inattendue");
            }
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════════════
//  T4 — la liste est DÉCLARÉE, et cohérente avec le registre
// ══════════════════════════════════════════════════════════════════════════════════════

#[test]
fn numeriques_est_declare_et_accorde_au_registre() {
    assert!(!NUMERIQUES.is_empty(), "une liste vide ferait passer ce garde sans rien garder");
    for (nom, min) in NUMERIQUES {
        let sw = switch(nom).unwrap_or_else(|| panic!("{nom} déclaré numérique mais absent du registre"));
        let d: u64 = sw
            .default
            .parse()
            .unwrap_or_else(|_| panic!("{nom} : le défaut « {} » du registre n'est pas un entier", sw.default));
        assert!(
            d >= *min,
            "{nom} : le défaut du registre ({d}) viole le minimum déclaré ({min}) — le repli serait \
             lui-même refusé, et le binaire tournerait sur une valeur que son propre rapport rejette"
        );
    }
}

#[test]
fn aucun_interrupteur_numerique_du_registre_nest_hors_de_la_liste() {
    // Le sens INVERSE de T4 : la liste ne doit pas se dériver des appels réels, mais elle ne doit pas
    // non plus oublier un interrupteur dont le registre annonce déjà un défaut numérique. Sans ce
    // garde, ajouter un second réglage chiffré passerait sous le filet, et le filet aurait un trou
    // qu'on croirait plein.
    for s in REGISTRY {
        let defaut_numerique = !s.default.is_empty() && s.default.parse::<u64>().is_ok();
        if defaut_numerique {
            assert!(
                NUMERIQUES.iter().any(|(n, _)| *n == s.name),
                "{} a un défaut numérique ({}) mais n'est pas dans NUMERIQUES : une valeur illisible \
                 y retomberait en silence",
                s.name,
                s.default
            );
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════════════
//  La contre-épreuve — un garde qui ne peut pas tomber ne garde rien
// ══════════════════════════════════════════════════════════════════════════════════════

#[test]
fn le_detecteur_sait_tomber_et_sait_se_taire() {
    // Le périmètre étant propre, les tests ci-dessus resteraient verts si `invalides_from_lookup`
    // rendait toujours un vecteur vide. On l'éprouve donc des deux côtés, sur une entrée synthétique.
    assert_eq!(invalides_from_lookup(table(&[(BAL, "pas un nombre")])).len(), 1, "doit voir");
    assert!(invalides_from_lookup(table(&[(BAL, "3600")])).is_empty(), "doit se taire");
    assert!(invalides_from_lookup(|_: &str| None).is_empty(), "et se taire sur rien");
}

#[test]
fn le_json_expose_le_troisieme_etat() {
    // Le rapport machine sert `--profile-report` ; taire les refus là où un outil lit reviendrait à
    // ne les avoir dits qu'à l'humain qui regarde le démarrage.
    let v = Report::from_lookup(table(&[(BAL, "1h")])).to_json();
    assert!(v["invalides"].is_array());
    assert_eq!(v["invalides"][0]["switch"], BAL);
    assert_eq!(v["invalides"][0]["fourni"], "1h");
    assert_eq!(v["invalides"][0]["effectif"], 3600);
}
