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
    entier_from_lookup, invalides_from_lookup, origines_from_lookup, switch, Report, ADRESSES,
    NUMERIQUES, REGISTRY,
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
//  M8 — la MÊME discipline pour une valeur qui n'est pas un nombre
// ══════════════════════════════════════════════════════════════════════════════════════
//
// `EFE_CONSOLE_TERMINATEUR_ORIGINE` est une liste d'adresses, et elle est le pire candidat possible
// à la substitution silencieuse : une adresse mal orthographiée ne dégrade pas un réglage de
// confort, elle DÉSACTIVE une vérification de sécurité — et l'exploitant, lui, croit l'avoir
// activée. C'est exactement l'écart entre l'intention et le résultat que M0 nomme.

const ORI: &str = "EFE_CONSOLE_TERMINATEUR_ORIGINE";

#[test]
fn une_adresse_illisible_est_refusee_et_non_appliquee() {
    let inv = invalides_from_lookup(table(&[(ORI, "pas-une-ip")]));
    assert_eq!(inv.len(), 1, "l'opérateur a agi, et son geste n'a aucun effet : c'est un refus");
    assert_eq!(inv[0].fourni, "pas-une-ip");
    assert_eq!(inv[0].effectif, 0, "aucune origine retenue — donc aucune corroboration");
    assert!(origines_from_lookup(ORI, table(&[(ORI, "pas-une-ip")])).is_empty());
}

/// **Le cas le plus dangereux** : la liste partiellement valide. Elle *paraît* marcher.
#[test]
fn une_liste_partiellement_illisible_est_signalee_et_non_tue() {
    let t = [(ORI, "10.0.0.5, 10.0.0.six, ::ffff:10.0.0.7")];
    let inv = invalides_from_lookup(table(&t));
    assert_eq!(inv.len(), 1, "une seule entrée fautive suffit à parler");
    assert_eq!(inv[0].fourni, "10.0.0.six", "et le rapport NOMME laquelle");
    assert_eq!(
        inv[0].effectif, 2,
        "deux origines seulement sont retenues — la corroboration marcherait pour deux chemins et \
         refuserait le troisième, et personne ne saurait pourquoi"
    );
    assert_eq!(origines_from_lookup(ORI, table(&t)).len(), 2, "servi = rapporté");
}

#[test]
fn une_liste_saine_ne_declenche_rien() {
    for v in ["10.0.0.5", "10.0.0.5,10.0.0.6", " 10.0.0.5 , ::ffff:10.0.0.6 ", "::1"] {
        assert!(
            invalides_from_lookup(table(&[(ORI, v)])).is_empty(),
            "« {v} » est recevable — le signaler crierait au loup"
        );
    }
    // Vide et absent ne sont PAS des refus : c'est une topologie non décrite, pas une faute.
    assert!(invalides_from_lookup(table(&[(ORI, "  ")])).is_empty());
    assert!(invalides_from_lookup(table(&[])).is_empty());
}

/// **Le défaut que ce garde fige.** `effectif` ne veut pas dire la même chose selon
/// l'interrupteur : pour un entier c'est *la valeur*, pour une liste c'est *un compte*. Le rendu
/// disait « le binaire emploie 1 » dans les deux cas — juste pour l'entier, **faux** pour la liste,
/// où il laissait croire que la valeur employée valait `1` au lieu de « une seule origine retenue ».
///
/// Un rapport qui dit autre chose que ce que fait le binaire est le défaut exact que M0 supprime.
/// Le champ `effectif` reste le nombre (machine, stable) ; `effectif_dit` porte la phrase.
#[test]
fn la_phrase_du_refus_dit_ce_que_le_nombre_signifie() {
    let entier = invalides_from_lookup(table(&[(BAL, "1h")]));
    assert_eq!(entier[0].effectif, 3600, "le champ machine reste un nombre");
    assert!(
        entier[0].effectif_dit.contains("la valeur employée reste 3600"),
        "pour un entier, le nombre EST la valeur : {}",
        entier[0].effectif_dit
    );

    let partielle = invalides_from_lookup(table(&[(ORI, "10.0.0.5, pas-une-ip")]));
    assert_eq!(partielle[0].effectif, 1, "le champ machine reste un nombre");
    assert!(
        !partielle[0].effectif_dit.contains("valeur employée"),
        "pour une liste, le nombre n'est PAS une valeur — la dire ainsi ment : {}",
        partielle[0].effectif_dit
    );
    assert!(
        partielle[0].effectif_dit.contains("1 origine(s) sur 2"),
        "il faut dire combien sur combien : {}",
        partielle[0].effectif_dit
    );

    // Le cas qui compte le plus : AUCUNE origine ne survit. « emploie 0 » serait illisible ;
    // l'exploitant doit lire que la corroboration n'aura pas lieu du tout.
    let nulle = invalides_from_lookup(table(&[(ORI, "pas-une-ip")]));
    assert!(
        nulle[0].effectif_dit.contains("aucune origine") && nulle[0].effectif_dit.contains("PAS"),
        "zéro origine doit se lire comme une absence de corroboration : {}",
        nulle[0].effectif_dit
    );

    // Et les deux formes atteignent le rendu HUMAIN, pas seulement la structure.
    let texte = Report::from_lookup(table(&[(ORI, "pas-une-ip")])).render_human();
    assert!(texte.contains("aucune origine"), "le rapport lu par l'humain le dit : {texte}");
    assert!(!texte.contains("emploie 0"), "et ne dit pas l'inverse : {texte}");
}

#[test]
fn adresses_est_declare_et_accorde_au_registre() {
    assert!(!ADRESSES.is_empty(), "une liste vide ferait passer ce garde sans rien garder");
    for nom in ADRESSES {
        let sw =
            switch(nom).unwrap_or_else(|| panic!("{nom} déclaré adresse mais absent du registre"));
        assert!(
            sw.default.is_empty(),
            "{nom} : un défaut non vide serait une origine qu'on n'a jamais déclarée — et donc une \
             corroboration que personne n'a demandée"
        );
    }
    // Les deux listes ne peuvent pas se recouvrir : `invalides_from_lookup` produirait deux refus
    // pour une seule valeur, et le rapport se contredirait lui-même.
    for (n, _) in NUMERIQUES {
        assert!(!ADRESSES.contains(n), "{n} ne peut pas être à la fois entier et adresse");
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
    // Idem sur le versant adresses : sans cette paire, la branche M8 pourrait rendre `None` en
    // toute circonstance et les quatre tests ci-dessus resteraient verts.
    assert_eq!(invalides_from_lookup(table(&[(ORI, "10.0.0.")])).len(), 1, "doit voir");
    assert!(invalides_from_lookup(table(&[(ORI, "10.0.0.5")])).is_empty(), "doit se taire");
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
