// SPDX-License-Identifier: UNLICENSED
//! # P1.a — un profil déclare ce qu'il **ne porte pas**.
//!
//! ## Le trou que ce fichier ferme
//!
//! Trois variables sont exigées par `frontier_boot_guard`, et aucune ne peut vivre dans un
//! `profiles/*.env` : `EFE_NANO_AUDIT_HMAC_MASTER` est un secret, `DATABASE_URL` en contient un, et
//! `EFE_OIDC_PUBLIC_KEY_PEM` est un blob **multiligne** qu'un fichier `.env` ne sait pas
//! représenter. `profile-check.sh` §2 interdit d'ailleurs déjà qu'un profil les pose.
//!
//! Il manquait la **contrepartie**. Avant P1, `profiles/frontiere.env` annonçait
//! `frontière : SERVIE` dans une configuration où le binaire fait `exit 78` — et rien ne pouvait
//! l'attraper : le garde vit dans `main.rs` sous `#[cfg(feature = "frontier-llm")]`, hors de portée
//! du chemin qui dérive un rapport depuis un fichier. **Aucun des trois — profil, attendu, garde —
//! ne regardait les deux autres.**
//!
//! ## La forme retenue, et pourquoi celle-là
//!
//! Une ligne `#!injection NOM` est **un commentaire** pour tout parseur `.env` (elle commence par
//! `#`) et **une déclaration** pour ce test. Le profil reste donc lisible par un opérateur, sans
//! second fichier à tenir à jour à côté — un fichier compagnon aurait été un deuxième dépôt de
//! vérité, exactement ce que M0 a supprimé.
//!
//! L'invariant est une **égalité**, pas une inclusion : ni moins (le profil mentirait par
//! omission), ni plus (il exigerait ce que le binaire ne demande pas, et l'opérateur poserait une
//! variable inutile en croyant qu'elle sert). Le jour où une quatrième précondition sera ajoutée au
//! garde, tout profil frontière devient rouge **le jour même**, au lieu d'être découvert en
//! `exit 78` par quelqu'un qui déploie.
//!
//! ## Ce que ce test ne prouve pas
//!
//! Que `INJECTIONS_FRONTIERE` corresponde au garde. Ce lien-là ne peut pas se faire ici — le garde
//! est dans le binaire — et il est tenu par
//! `p1_chaque_injection_declaree_est_une_precondition_du_garde`, dans les tests de `main.rs`, qui
//! exige en outre que le message de refus **nomme** la variable.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use efe_swarm_gateway::profile::INJECTIONS_FRONTIERE;

fn profiles_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles")
}

/// Le marqueur, isolé — pour que le lecteur ait sa propre contre-épreuve (voir plus bas).
///
/// Volontairement strict : `#!injection` puis **un seul** nom. Une syntaxe permissive inviterait à
/// écrire des listes, des commentaires en fin de ligne, des espaces significatifs — et un jour à
/// lire de travers une déclaration qui décide si un egress payant démarre.
fn injections_declarees(texte: &str) -> BTreeSet<String> {
    let mut v = BTreeSet::new();
    for l in texte.lines() {
        let l = l.trim();
        let Some(reste) = l.strip_prefix("#!injection") else { continue };
        let nom = reste.trim();
        assert!(
            !nom.is_empty() && !nom.contains(char::is_whitespace),
            "`#!injection` doit être suivi d'UN nom de variable, et d'un seul : « {l} »"
        );
        assert!(v.insert(nom.to_string()), "injection déclarée deux fois : {nom}");
    }
    v
}

/// La frontière est-elle demandée par ce texte de profil ? Lecture minimale et stricte, comme le
/// parseur du test de profils : seule la chaîne exacte `true` active un drapeau.
fn frontiere_demandee(texte: &str) -> bool {
    texte.lines().any(|l| l.trim() == "EFE_LLM_FRONTIER_ENABLED=true")
}

fn profils() -> Vec<(String, String)> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(profiles_dir()).expect("profiles/ lisible").flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "env") {
            continue;
        }
        let nom = p.file_stem().expect("tige").to_string_lossy().to_string();
        let texte = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{nom}.env : {e}"));
        v.push((nom, texte));
    }
    assert!(v.len() >= 5, "seulement {} profils trouvés — le test se croirait vert à tort", v.len());
    v
}

/// ⭐⭐ **L'invariant** : frontière demandée ⟺ les injections déclarées sont EXACTEMENT celles que
/// le binaire exige.
#[test]
fn p1_un_profil_frontiere_declare_exactement_les_injections_du_garde() {
    let attendu: BTreeSet<String> = INJECTIONS_FRONTIERE.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(
        attendu.len(),
        INJECTIONS_FRONTIERE.len(),
        "INJECTIONS_FRONTIERE contient un doublon"
    );

    let mut vus = 0usize;
    for (nom, texte) in profils() {
        let declarees = injections_declarees(&texte);
        if !frontiere_demandee(&texte) {
            // Un profil sans frontière n'a rien à déclarer. Tolérer une déclaration ici viderait le
            // marqueur de son sens : il ne signifierait plus « le garde l'exige ».
            assert!(
                declarees.is_empty(),
                "profiles/{nom}.env déclare des injections {declarees:?} sans activer la frontière \
                 — le marqueur ne veut plus rien dire s'il sert aussi à autre chose"
            );
            continue;
        }
        vus += 1;
        assert_eq!(
            declarees, attendu,
            "\nprofiles/{nom}.env active la frontière, et sa déclaration d'injections diverge de ce \
             que le binaire EXIGE.\n  déclaré : {declarees:?}\n  exigé   : {attendu:?}\n\n\
             Ni moins (le profil mentirait par omission — l'opérateur découvrirait le manque en \
             exit 78),\nni plus (il ferait poser une variable que le binaire ne demande pas).\n"
        );
    }
    assert!(
        vus > 0,
        "aucun profil n'active la frontière — l'invariant n'a porté sur rien. Si c'est voulu, \
         supprimer ce test plutôt que de le laisser vert par vacuité."
    );
}

/// Un profil qui active la frontière est **toujours** rendu « REFUSÉE » quand on le lit seul.
///
/// C'est le pendant du test précédent, côté rapport : puisqu'un `.env` ne porte jamais les
/// injections, il ne peut jamais servir la frontière. Ce test échouerait si quelqu'un remettait un
/// jour la ligne d'origine (`line(..., r.frontier)`), qui disait « SERVIE » sur le seul drapeau.
#[test]
fn p1_un_profil_seul_ne_sert_jamais_la_frontiere() {
    for (nom, texte) in profils() {
        if !frontiere_demandee(&texte) {
            continue;
        }
        let vars: std::collections::BTreeMap<String, String> = texte
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect();
        let r = efe_swarm_gateway::profile::Resolved::from_lookup(|k| vars.get(k).cloned());
        let rapport = efe_swarm_gateway::profile::Report::from_resolved(&r);
        let json = rapport.to_json();
        let statut = json["capabilities"]
            .as_array()
            .expect("capabilities")
            .iter()
            .find(|c| c["capability"] == "frontière")
            .map(|c| c["status"].as_str().unwrap_or("").to_string())
            .expect("la ligne « frontière » existe");
        assert_eq!(
            statut, "REFUSÉE",
            "profiles/{nom}.env rend « frontière : {statut} » alors qu'il ne porte aucune \
             injection. Le mot SERVIE a un sens unique dans ce dépôt ; l'étirer à « serait servie \
             si les secrets étaient là » rouvre l'écart déclaré/servi que M0 ferme."
        );
    }
}

// ─── Les contre-épreuves : le lecteur voit-il ce qu'il prétend voir ? ───────────────────────
//
// Sans elles, les deux tests ci-dessus pourraient être verts parce que `injections_declarees` rend
// toujours l'ensemble vide — et l'invariant ne porterait sur rien. C'est le défaut que M7 avait
// laissé (`seules_les_deux_sondes_sont_publiques` ne pouvait pas tomber) et que M1.b a corrigé en
// isolant son détecteur.

#[test]
fn p1_le_lecteur_de_marqueurs_voit_ce_quil_pretend_voir() {
    let t = "# un commentaire\n#!injection A\nCLE=valeur\n#!injection B\n#pas un marqueur\n";
    let d = injections_declarees(t);
    assert_eq!(d.len(), 2, "le lecteur n'a pas vu les deux marqueurs : {d:?}");
    assert!(d.contains("A") && d.contains("B"));

    // Un texte sans marqueur en rend zéro — et non « tout ce qui commence par # ».
    assert!(injections_declarees("# rien ici\nCLE=v\n").is_empty());
    // Un `#!injection` en fin de ligne de code n'est PAS un marqueur : seule une ligne entière l'est.
    assert!(injections_declarees("CLE=v # !injection A\n").is_empty());
    // La casse et l'espacement du préfixe ne sont pas tolérés — un marqueur approximatif serait
    // silencieusement ignoré, et le profil paraîtrait conforme.
    assert!(injections_declarees("# !injection A\n").is_empty());
    assert!(injections_declarees("#!INJECTION A\n").is_empty());
}

#[test]
fn p1_le_lecteur_de_drapeau_distingue_true_du_reste() {
    assert!(frontiere_demandee("EFE_LLM_FRONTIER_ENABLED=true\n"));
    assert!(!frontiere_demandee("EFE_LLM_FRONTIER_ENABLED=false\n"));
    assert!(!frontiere_demandee("EFE_LLM_FRONTIER_ENABLED=1\n"));
    assert!(!frontiere_demandee("# EFE_LLM_FRONTIER_ENABLED=true\n"));
    assert!(!frontiere_demandee("EFE_LLM_FRONTIER_ENABLED=TRUE\n"));
}

/// Et la contre-épreuve de l'invariant lui-même : une déclaration amputée doit être REFUSÉE.
///
/// On ne peut pas muter un profil livré depuis un test ; on rejoue donc la comparaison sur des
/// ensembles synthétiques, avec la même règle d'égalité.
#[test]
fn p1_une_declaration_amputee_ou_gonflee_est_refusee() {
    let attendu: BTreeSet<String> = INJECTIONS_FRONTIERE.iter().map(|s| (*s).to_string()).collect();

    let mut ampute = attendu.clone();
    let retire = ampute.iter().next().expect("non vide").clone();
    ampute.remove(&retire);
    assert_ne!(ampute, attendu, "retirer {retire} doit faire diverger");

    let mut gonfle = attendu.clone();
    gonfle.insert("EFE_UNE_VARIABLE_QUE_LE_GARDE_NIGNORE_PAS".to_string());
    assert_ne!(gonfle, attendu, "ajouter une variable doit faire diverger");

    assert_eq!(attendu, attendu.clone(), "et l'égalité exacte doit passer");
}
