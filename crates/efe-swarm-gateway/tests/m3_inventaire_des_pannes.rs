// SPDX-License-Identifier: UNLICENSED
//! **M3 — l'inventaire des pannes, rendu vérifiable.**
//!
//! ## Le trou que M3 avait DÉCLARÉ, et que ce fichier ferme
//!
//! M3 a livré l'inventaire des pannes du chemin d'appel — ce qui dégrade, ce qui coupe, et
//! pourquoi — puis a écrit sa propre limite, mot pour mot :
//!
//! > *« Cet inventaire est tenu à la main. Aucun test n'interdit qu'une dépendance future coupe sans
//! > être inscrite ici — un `?` sur un appel faillible suffirait. Les lignes ci-dessus gardent contre
//! > la RÉGRESSION, pas contre l'OUBLI. Le dire vaut mieux que laisser croire à un filet qui
//! > n'existe pas. »*
//!
//! **Ce n'était pas une inquiétude théorique : l'oubli avait déjà eu lieu.** La doc de `capture.rs`
//! le consigne — \[M5\] avait ajouté une dépendance sur le chemin d'appel *sans inscrire sa ligne*,
//! et l'inventaire était périmé un module après avoir été écrit. C'est-à-dire exactement le mode de
//! panne qu'il dénonce chez les autres.
//!
//! ## Ce que ce garde vérifie, et ce qu'il ne peut pas vérifier
//!
//! Il **ne peut pas** répondre à la question qui décide un régime — *dégrader produit-il quelque
//! chose d'utile et d'honnête ?* Cette question demande de savoir ce qui reste au client quand la
//! dépendance meurt, et aucune machine ne le lit dans le code.
//!
//! Il peut, en revanche, garantir les trois propriétés mécaniques sans lesquelles la réponse humaine
//! ne vaut rien :
//!
//! 1. **chaque preuve citée existe** — sinon l'inventaire renvoie à un test fantôme ;
//! 2. **le tableau de la doc et cette table disent la même chose** — sinon la prose et la donnée
//!    divergent, et c'est la prose qu'on lit ;
//! 3. **aucune dépendance du chemin servi n'est hors inventaire** — c'est le garde anti-oubli, celui
//!    que M3 déclarait ne pas avoir, et celui qui aurait attrapé M5.
//!
//! ## Deux dépendances manquaient
//!
//! Écrire (3) l'a immédiatement prouvé utile : `efe_swarm_sdk` et `efe_swarm_provenance` sont
//! importés par le chemin servi et n'avaient **aucune ligne**. Toutes deux sont infaillibles — mais
//! l'inventaire a précisément une ligne pour ce cas (« caviardage PII | infaillible (pas de
//! `Result`) »), donc leur absence n'était pas une exemption : c'était un oubli. Le silence sur une
//! dépendance ne dit pas « elle ne peut pas échouer », il ne dit rien.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Le régime d'une dépendance face à sa propre panne.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Regime {
    /// La panne laisse quelque chose d'utile, et l'aveu la rend honnête.
    Degrade,
    /// Le pair fautif sort du fan-out ; l'arité déclarée reste re-dérivable.
    Ecarte,
    /// Il n'y a rien vers quoi dégrader : couper est la seule réponse qui ne mente pas.
    Coupe,
    /// Elle ne rend pas de `Result` — la question du régime ne se pose pas.
    Infaillible,
    /// Au mieux, et borné : la perte est comptée, jamais propagée.
    BestEffort,
}

struct Ligne {
    dependance: &'static str,
    regime: Regime,
    /// Le crate qui la fournit, quand c'en est un. C'est par ce champ que le garde anti-oubli fait
    /// la correspondance avec les imports réels du chemin servi. `None` = dépendance interne au
    /// crate (une couture, pas une bibliothèque), que l'analyse d'imports ne verra jamais.
    fournie_par: Option<&'static str>,
    /// Le test qui l'établit, ou la raison quand il n'y a rien à tester.
    preuve: &'static str,
}

/// L'inventaire — la même table que la doc de `capture.rs`, sous forme de donnée.
const INVENTAIRE: &[Ligne] = &[
    Ligne {
        dependance: "reformulateur SLM mort",
        regime: Regime::Degrade,
        fournie_par: Some("efe_multi_llm"),
        preuve: "m3_le_slm_mort_ne_coupe_pas_et_sa_raison_remonte",
    },
    Ligne {
        dependance: "store (persistance)",
        regime: Regime::Degrade,
        fournie_par: None,
        preuve: "m2_une_mission_survit_a_la_mort_du_store",
    },
    Ligne {
        dependance: "les deux à la fois",
        regime: Regime::Degrade,
        fournie_par: None,
        preuve: "m3_le_slm_et_le_store_meurent_ensemble_et_lappel_passe",
    },
    Ligne {
        dependance: "provider frontière mort",
        regime: Regime::Ecarte,
        fournie_par: Some("efe_coordinator"),
        preuve: "call_many_ok_drops_errors",
    },
    Ligne {
        dependance: "dérivation réglementaire (M5)",
        regime: Regime::Degrade,
        fournie_par: None,
        preuve: "m5_un_dossier_manque_se_rattrape_a_lidentique",
    },
    Ligne {
        dependance: "caviardage PII",
        regime: Regime::Infaillible,
        fournie_par: None,
        preuve: "pas de `Result` — il n'y a pas de panne à traiter",
    },
    Ligne {
        dependance: "bus de supervision",
        regime: Regime::BestEffort,
        fournie_par: Some("efe_swarm_dashboard"),
        preuve: "ring borné + compteur de pertes — la perte est comptée, jamais propagée",
    },
    Ligne {
        dependance: "tenant_id",
        regime: Regime::Coupe,
        fournie_par: None,
        preuve: "entrée du CLIENT : invalide, c'est une requête malformée, pas une panne",
    },
    Ligne {
        dependance: "derive_key",
        regime: Regime::Coupe,
        fournie_par: None,
        preuve: "sans clé rien ne peut être scellé : dégrader rendrait une réponse sans aucun reçu \
                 possible, ni maintenant ni plus tard",
    },
    Ligne {
        dependance: "publication d'ancre",
        regime: Regime::Coupe,
        fournie_par: None,
        preuve: "annoncer `anchored: true` après un échec serait un mensonge sur la garantie",
    },
    // ── Les deux lignes que le garde anti-oubli a fait apparaître ────────────────────────
    Ligne {
        dependance: "mémoire d'essaim (MemoryHandle)",
        regime: Regime::Infaillible,
        fournie_par: Some("efe_swarm_sdk"),
        preuve: "`MemoryHandle::with_config` construit un descripteur, sans `Result` : il n'y a pas \
                 de panne à ce point du chemin",
    },
    Ligne {
        dependance: "re-preuve hors ligne et empreintes",
        regime: Regime::Infaillible,
        fournie_par: Some("efe_swarm_provenance"),
        preuve: "`blake3_bytes` rend une empreinte et `verify_run_offline` rend un VERDICT — un \
                 verdict négatif n'est pas une panne, c'est le résultat que le client demandait",
    },
];

/// Les crates que le chemin servi importe et qui ne peuvent structurellement pas être une
/// dépendance faillible — avec la raison, jamais une liste nue.
const SANS_OBJET: &[(&str, &str)] = &[
    ("std", "la bibliothèque standard n'est pas une dépendance de service"),
    ("serde", "sérialisation en mémoire : une entrée malformée est une requête malformée, traitée \
               par la ligne `tenant_id` du même inventaire"),
    ("serde_json", "même raison que `serde` — et c'est le garde lui-même qui l'a réclamé : la \
                    première rédaction de cette liste avait exempté la famille sans nommer ce \
                    membre-là, ce qui est la forme habituelle de l'oubli qu'on prétend traquer"),
    ("axum", "le cadre HTTP lui-même — s'il tombe, il n'y a plus de chemin servi du tout, donc plus \
              rien à dégrader vers"),
    ("tokio", "l'exécuteur : même raisonnement que le cadre HTTP"),
    ("uuid", "génération et analyse d'identifiants, sans entrée-sortie"),
    ("crate", "les coutures internes portent leur propre ligne quand elles en méritent une — le \
               champ `fournie_par` vaut alors `None`"),
    ("super", "idem : chemin interne au module"),
];

/// Les fichiers du **chemin d'appel** — ceux que l'inventaire dit couvrir : les deux voies qui
/// persistent, l'essaim et la mission frontière.
const CHEMIN_SERVI: &[&str] = &[
    "crates/efe-swarm-gateway/src/routes/run.rs",
    "crates/efe-swarm-gateway/src/routes/llm.rs",
];

fn racine() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("racine du dépôt")
}

/// Le code **servi** d'un fichier : tout ce qui précède son module de tests.
///
/// Un `use` qui n'existe que pour les tests n'est pas une dépendance de service, et l'inscrire à
/// l'inventaire ferait croire à une surface qui n'est pas exposée au client.
fn region_servie(src: &str) -> &str {
    src.find("#[cfg(test)]").map_or(src, |i| &src[..i])
}

/// Les crates racines que ce source importe.
fn crates_importes(src: &str) -> BTreeSet<String> {
    src.lines()
        .map(str::trim_start)
        .filter_map(|l| l.strip_prefix("use "))
        .filter_map(|reste| reste.split(&[':', ' ', '{', ';'][..]).next())
        .filter(|c| !c.is_empty())
        .map(str::to_string)
        .collect()
}

// ══════════════════════════════════════════════════════════════════════════════════════
//  (1) Chaque preuve citée existe — sinon l'inventaire renvoie à un fantôme
// ══════════════════════════════════════════════════════════════════════════════════════

#[test]
fn chaque_preuve_nommee_existe_reellement() {
    let racine = racine();
    let mut sources = String::new();
    for e in walkdir(&racine.join("crates")) {
        if e.extension().is_some_and(|x| x == "rs") {
            if let Ok(t) = std::fs::read_to_string(&e) {
                sources.push_str(&t);
            }
        }
    }
    let mut manquants = Vec::new();
    for l in INVENTAIRE {
        // Une preuve qui ressemble à un nom de test DOIT en être un ; une preuve rédigée en prose
        // est une justification, et se juge à la lecture.
        let ressemble_a_un_test = !l.preuve.contains(' ') && l.preuve.contains('_');
        if ressemble_a_un_test && !sources.contains(&format!("fn {}", l.preuve)) {
            manquants.push(format!("  {} → « {} » n'existe pas", l.dependance, l.preuve));
        }
    }
    assert!(
        manquants.is_empty(),
        "L'inventaire des pannes cite des preuves qui n'existent plus :\n{}\n\nUn inventaire qui \
         renvoie à un test fantôme est pire qu'un inventaire absent : il donne l'assurance sans la \
         garantie.",
        manquants.join("\n")
    );
}

// ══════════════════════════════════════════════════════════════════════════════════════
//  (2) LE GARDE ANTI-OUBLI — celui que M3 déclarait ne pas avoir
// ══════════════════════════════════════════════════════════════════════════════════════

#[test]
fn aucune_dependance_du_chemin_servi_nest_hors_inventaire() {
    let racine = racine();
    let connus: BTreeSet<&str> = INVENTAIRE.iter().filter_map(|l| l.fournie_par).collect();
    let exemptes: BTreeSet<&str> = SANS_OBJET.iter().map(|(c, _)| *c).collect();

    let mut orphelines = Vec::new();
    let mut vus = 0usize;
    for rel in CHEMIN_SERVI {
        let src = std::fs::read_to_string(racine.join(rel))
            .unwrap_or_else(|e| panic!("{rel} illisible : {e} — le chemin servi a-t-il déménagé ?"));
        for c in crates_importes(region_servie(&src)) {
            vus += 1;
            if !connus.contains(c.as_str()) && !exemptes.contains(c.as_str()) {
                orphelines.push(format!("  {rel} importe « {c} »"));
            }
        }
    }
    assert!(vus > 5, "seulement {vus} imports vus — l'analyse ne lit manifestement pas le chemin servi");
    assert!(
        orphelines.is_empty(),
        "Une dépendance du chemin d'appel n'est PAS à l'inventaire des pannes :\n{}\n\n\
         Inscrivez-la — avec son régime et pourquoi. Si elle est infaillible, dites-le : le silence \
         sur une dépendance ne dit pas « elle ne peut pas échouer », il ne dit RIEN. Si elle peut \
         couper, la seule question qui décide est : dégrader produirait-il quelque chose d'utile ET \
         d'honnête ?\n\n\
         C'est ce garde précis que M3 déclarait ne pas avoir, et c'est ainsi que M5 avait ajouté une \
         dépendance sans inscrire sa ligne.",
        orphelines.join("\n")
    );
}

// ══════════════════════════════════════════════════════════════════════════════════════
//  (3) La prose et la donnée disent la même chose
// ══════════════════════════════════════════════════════════════════════════════════════

#[test]
fn le_tableau_de_la_doc_et_linventaire_saccordent() {
    // Les accents graves sont retirés des DEUX côtés : `MemoryHandle` et MemoryHandle sont le même
    // mot, et un garde qui tomberait sur une mise en forme apprendrait au lecteur à l'ignorer. Ce
    // qu'on veut voir tomber, c'est une REFORMULATION — pas un balisage markdown.
    let doc = std::fs::read_to_string(racine().join("crates/efe-swarm-gateway/src/capture.rs"))
        .expect("capture.rs lisible")
        .replace('`', "");
    let mut absentes = Vec::new();
    for l in INVENTAIRE {
        // On cherche la dépendance telle que la table la nomme. La comparaison est volontairement
        // littérale : si quelqu'un reformule la ligne d'un côté seulement, les deux divergent, et
        // c'est exactement ce qu'on veut voir tomber.
        if !doc.contains(&l.dependance.replace('`', "")) {
            absentes.push(format!("  « {} » est dans la donnée, pas dans le tableau", l.dependance));
        }
    }
    assert!(
        absentes.is_empty(),
        "Le tableau de `capture.rs` et l'inventaire exécutable ont divergé :\n{}\n\nC'est la prose \
         qu'un lecteur lit ; une donnée qui la contredit ne le protège de rien.",
        absentes.join("\n")
    );
}

#[test]
fn chaque_ligne_dit_un_regime_et_une_raison() {
    for l in INVENTAIRE {
        assert!(!l.dependance.trim().is_empty());
        assert!(
            l.preuve.trim().len() > 10,
            "{} : une preuve doit dire quelque chose — « {} » ne dit rien",
            l.dependance,
            l.preuve
        );
        // Une coupure est la seule décision qui prive le client de tout. Elle exige une raison
        // rédigée, jamais un nom de test : un test prouve QUE ça coupe, pas que couper est juste.
        if l.regime == Regime::Coupe {
            assert!(
                l.preuve.contains(' '),
                "{} coupe : il faut dire POURQUOI en toutes lettres, pas citer un test",
                l.dependance
            );
        }
    }
    for (c, pourquoi) in SANS_OBJET {
        assert!(pourquoi.len() > 30, "l'exemption de « {c} » doit dire pourquoi, pas seulement qu'elle existe");
    }
}

// ══════════════════════════════════════════════════════════════════════════════════════
//  La contre-épreuve — un garde qui ne peut pas tomber ne garde rien
// ══════════════════════════════════════════════════════════════════════════════════════

#[test]
fn les_detecteurs_savent_tomber_et_savent_se_taire() {
    // Le périmètre étant propre, les trois gardes ci-dessus resteraient verts même si leurs
    // extracteurs étaient cassés. On les éprouve donc sur des entrées synthétiques.
    let src = "use efe_inconnu::Chose;\nuse std::fmt;\n#[cfg(test)]\nuse efe_de_test::Truc;\n";
    let servi = region_servie(src);
    let vus = crates_importes(servi);
    assert!(vus.contains("efe_inconnu"), "l'extracteur doit voir un import servi");
    assert!(vus.contains("std"));
    assert!(
        !vus.contains("efe_de_test"),
        "un import qui n'existe que pour les tests n'est pas une dépendance de service"
    );
    assert!(crates_importes("fn rien() {}").is_empty(), "et se taire quand il n'y a rien");
}

/// Parcours récursif — `walkdir` n'est pas une dépendance de ce crate, et l'ajouter pour un test
/// serait payer une bibliothèque pour six lignes.
fn walkdir(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entrees) = std::fs::read_dir(dir) else { return out };
    for e in entrees.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            out.extend(walkdir(&p));
        } else {
            out.push(p);
        }
    }
    out
}
