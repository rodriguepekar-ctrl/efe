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
    /// Ce qui la fournit sur le chemin servi — un crate (`efe_multi_llm`) ou une couture interne
    /// (`crate::capture`). C'est par ce champ que le garde anti-oubli fait la correspondance.
    ///
    /// Une **liste** et non une valeur : le store est atteint par deux modules, et forcer un seul
    /// nom obligerait à en taire un.
    fournie_par: &'static [&'static str],
    /// Le test qui l'établit, ou la raison quand il n'y a rien à tester.
    preuve: &'static str,
}

/// L'inventaire — la même table que la doc de `capture.rs`, sous forme de donnée.
const INVENTAIRE: &[Ligne] = &[
    Ligne {
        dependance: "reformulateur SLM mort",
        regime: Regime::Degrade,
        fournie_par: &["efe_multi_llm"],
        preuve: "m3_le_slm_mort_ne_coupe_pas_et_sa_raison_remonte",
    },
    Ligne {
        dependance: "store (persistance)",
        regime: Regime::Degrade,
        fournie_par: &["crate::capture", "crate::store"],
        preuve: "m2_une_mission_survit_a_la_mort_du_store",
    },
    Ligne {
        dependance: "les deux à la fois",
        regime: Regime::Degrade,
        fournie_par: &[],
        preuve: "m3_le_slm_et_le_store_meurent_ensemble_et_lappel_passe",
    },
    Ligne {
        dependance: "provider frontière mort",
        regime: Regime::Ecarte,
        fournie_par: &["efe_coordinator"],
        preuve: "call_many_ok_drops_errors",
    },
    Ligne {
        dependance: "dérivation réglementaire (M5)",
        regime: Regime::Degrade,
        fournie_par: &["crate::reg_pont"],
        preuve: "m5_un_dossier_manque_se_rattrape_a_lidentique",
    },
    Ligne {
        dependance: "caviardage PII",
        regime: Regime::Infaillible,
        fournie_par: &["crate::pii"],
        preuve: "pas de `Result` — il n'y a pas de panne à traiter",
    },
    Ligne {
        dependance: "bus de supervision",
        regime: Regime::BestEffort,
        fournie_par: &["efe_swarm_dashboard"],
        preuve: "ring borné + compteur de pertes — la perte est comptée, jamais propagée",
    },
    Ligne {
        dependance: "tenant_id",
        regime: Regime::Coupe,
        fournie_par: &["crate::auth"],
        preuve: "entrée du CLIENT : invalide, c'est une requête malformée, pas une panne",
    },
    Ligne {
        dependance: "derive_key",
        regime: Regime::Coupe,
        fournie_par: &[],
        preuve: "sans clé rien ne peut être scellé : dégrader rendrait une réponse sans aucun reçu \
                 possible, ni maintenant ni plus tard",
    },
    Ligne {
        dependance: "publication d'ancre",
        regime: Regime::Coupe,
        // Cet index était VIDE : la ligne existait, rien ne la reliait au code. Le garde ne
        // regardait alors que `run.rs` et `llm.rs`, et `bundle.rs` — qui appelle `anchor_of` —
        // n'était pas lu. Une ligne sans index n'attrape rien : elle décore.
        fournie_par: &["provenance_bridge"],
        preuve: "annoncer `anchored: true` après un échec serait un mensonge sur la garantie",
    },
    // ── Les deux lignes que le garde anti-oubli a fait apparaître ────────────────────────
    Ligne {
        dependance: "mémoire d'essaim (MemoryHandle)",
        regime: Regime::Infaillible,
        fournie_par: &["efe_swarm_sdk", "efe_swarm_memory"],
        preuve: "`MemoryHandle::with_config` construit un descripteur, sans `Result` : il n'y a pas \
                 de panne à ce point du chemin. Le second point d'entrée, `verify_memory` \
                 (routes/memory.rs), rend un `MemoryChainVerdict` et non un `Result` — une chaîne \
                 rompue y est un VERDICT, pas une panne, et le distinguer est tout l'intérêt",
    },
    Ligne {
        dependance: "types de tenant (efe-nano-sdk)",
        regime: Regime::Infaillible,
        fournie_par: &["efe_nano_sdk"],
        // Une exemption globale aurait été la faute de M1.b : « sûr par composition » couvre alors
        // en silence un usage futur qui ne l'est pas. Une LIGNE, elle, est visible dans l'artefact
        // que lit un auditeur, et sa prose nomme le site exact — de sorte qu'un appel faillible
        // rendrait ce texte visiblement faux au lieu de passer inaperçu.
        preuve: "un seul site sur tout le chemin servi — `routes/events.rs`, en position de TYPE : \
                 `&efe_nano_sdk::TenantId`, un `struct TenantId(pub Uuid)`. Il n'y a rien à \
                 joindre, donc rien à dégrader. Le jour où une fonction faillible du SDK est \
                 appelée, cette phrase cesse d'être vraie et doit devenir une autre ligne",
    },
    Ligne {
        dependance: "re-preuve hors ligne et empreintes",
        regime: Regime::Infaillible,
        fournie_par: &["efe_swarm_provenance"],
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
    ("async_stream", "la macro `stream!` du motif SSE d'axum 0.7 : elle CONSTRUIT un flux, elle ne \
                      joint rien. Même famille qu'`axum` et `tokio` — si elle manquait, il n'y \
                      aurait pas de chemin servi à dégrader"),
    ("futures", "le trait `Stream`, importé pour un type de retour. Un trait ne tombe pas"),
    ("crate", "les coutures internes portent leur propre ligne quand elles en méritent une — le \
               champ `fournie_par` vaut alors `None`"),
    ("super", "idem : chemin interne au module"),
    ("header", "`axum::http::header` — des constantes de nom d'en-tête, sans entrée-sortie ni \
                `Result`"),
    ("crate::error", "le TYPE d'erreur de la passerelle. Il transporte les coupures des autres, il \
                      n'en produit aucune"),
    ("crate::dto", "des structures de transport : sérialisation en mémoire, déjà couverte par la \
                    ligne `tenant_id` pour ce qui est des entrées malformées"),
    ("crate::state", "l'état partagé porte les descripteurs des dépendances ; ce sont ELLES qui ont \
                      une ligne, pas le sac qui les tient"),
    ("crate::profile", "le registre des interrupteurs (M0) : une seule décision, au démarrage, avec \
                        défaut déclaré — rien à dégrader au moment d'une requête"),
    ("crate::mission", "la plomberie de la mission frontière — c'est le SUJET des lignes ci-dessus \
                        (SLM, store, providers), pas une dépendance de plus"),
    ("crate::mission_graph", "idem, et c'est ce module qui porte `m2_une_mission_survit_a_la_mort_du_store` : \
                              il est le lieu du test, pas une dépendance à inventorier"),
];

/// Les fichiers du **chemin d'appel** — **découverts**, jamais énumérés.
///
/// Ils l'étaient : une liste de deux, `routes/run.rs` et `routes/llm.rs`, les deux voies qui
/// persistent. Le raisonnement se tenait — ce sont elles qui portent la mission payante — mais il
/// laissait le garde surveiller **2 fichiers sur 14**. Les douze autres atteignent sept crates du
/// dépôt (`provenance_bridge`, `efe_swarm_dashboard`, `efe_nano_sdk`, `efe_swarm_memory`,
/// `efe_swarm_provenance`, `efe_swarm_sdk`, `reg_evidence` — celui de C1) et une dépendance ajoutée
/// dans `bundle.rs` ou `reg.rs` n'aurait jamais eu à se déclarer.
///
/// C'est la faute exacte que ce garde existe pour empêcher, commise par le garde lui-même : une
/// liste écrite à la main ne voit que ce qu'on a bien voulu lui montrer, et le jour où elle a tort
/// est le jour où quelqu'un ajoute quelque chose. Le dossier est donc **lu**.
fn chemin_servi(racine: &Path) -> Vec<PathBuf> {
    let dir = racine.join("crates/efe-swarm-gateway/src/routes");
    let mut v: Vec<PathBuf> = walkdir(&dir)
        .into_iter()
        .filter(|e| e.extension().is_some_and(|x| x == "rs"))
        .collect();
    v.sort();
    v
}

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
fn region_servie(src: &str) -> String {
    let sans_tests = src.find("#[cfg(test)]").map_or(src, |i| &src[..i]);
    // Les commentaires sont retirés, et ce n'est pas de la coquetterie : `llm.rs` cite
    // `crate::oidc::OidcVerifier` dans sa documentation de module, alors que le code servi ne
    // l'appelle jamais. Un garde qui lit la prose comme du code réclame une ligne d'inventaire pour
    // une dépendance imaginaire — et le bruit fait ajouter des lignes creuses, ou désactiver le garde.
    sans_tests
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Les dépendances qu'un source atteint — **l'union de deux extracteurs**, parce qu'aucun des deux
/// seul ne suffit.
///
/// * les lignes `use` attrapent `use efe_multi_llm::{A, B}`, que la seconde méthode rate : après
///   `::` vient une accolade, pas un identifiant ;
/// * les chemins **pleinement qualifiés** attrapent `crate::reg_pont::RegReport`, qu'aucun `use`
///   ne déclare.
///
/// La première rédaction de ce garde n'avait que la première méthode, et exemptait `crate` en bloc.
/// Elle ratait donc **deux fois** la dépendance de \[M5\] — celle-là même que son message d'échec
/// citait en exemple. Un garde qui rate le cas qu'il donne pour motif est pire qu'absent : il
/// rassure.
///
/// Un chemin interne compte pour son **module** (`crate::reg_pont`), pas pour le mot `crate` : c'est
/// le module qui est la dépendance. Les racines en majuscule sont écartées — `Uuid::new_v4()` est un
/// appel de fonction associée, pas un chemin de module.
fn dependances(src: &str) -> BTreeSet<String> {
    const PRIMITIFS: &[&str] =
        &["std", "core", "alloc", "self", "super", "crate",
          // `u32::try_from(..)` est une conversion, pas une dépendance de service.
          "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "isize", "f32", "f64", "bool", "char", "str"];
    // Les sous-modules DÉCLARÉS dans ce même source ne sont pas des crates. `routes/mod.rs` écrit
    // `pub mod bundle;` puis `bundle::poser(...)` : sans cette lecture, l'assemblage du routeur
    // réclamerait une ligne d'inventaire pour chacune de ses propres routes. Le fait est tiré du
    // fichier lui-même, pas d'une liste — donc il ne peut pas se démoder.
    let modules: BTreeSet<String> = src
        .lines()
        .filter_map(|l| {
            let t = l.trim_start();
            let r = t.strip_prefix("pub mod ").or_else(|| t.strip_prefix("mod "))?;
            r.split([';', ' ', '{']).next().filter(|m| !m.is_empty()).map(str::to_string)
        })
        .collect();

    let mut out = BTreeSet::new();

    // (1) les `use`
    for l in src.lines().map(str::trim_start) {
        let Some(reste) = l.strip_prefix("use ") else { continue };
        let racine = reste.split([':', ' ', '{', ';']).next().unwrap_or("");
        if racine == "crate" {
            if let Some(m) = reste.strip_prefix("crate::").and_then(|r| r.split([':', ' ', '{', ';', ',']).next()) {
                if !m.is_empty() {
                    out.insert(format!("crate::{m}"));
                }
            }
        } else if !racine.is_empty() && !PRIMITIFS.contains(&racine) && !modules.contains(racine) {
            out.insert(racine.to_string());
        }
    }

    // (2) les chemins pleinement qualifiés
    let octets: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    while i < octets.len() {
        if !(octets[i].is_ascii_alphabetic() || octets[i] == '_')
            || (i > 0 && (octets[i - 1].is_alphanumeric() || octets[i - 1] == '_' || octets[i - 1] == ':'))
        {
            i += 1;
            continue;
        }
        let debut = i;
        while i < octets.len() && (octets[i].is_alphanumeric() || octets[i] == '_') {
            i += 1;
        }
        if i + 1 >= octets.len() || octets[i] != ':' || octets[i + 1] != ':' {
            continue;
        }
        let tete: String = octets[debut..i].iter().collect();
        if tete == "crate" {
            let d2 = i + 2;
            let mut j = d2;
            while j < octets.len() && (octets[j].is_alphanumeric() || octets[j] == '_') {
                j += 1;
            }
            let m: String = octets[d2..j].iter().collect();
            if !m.is_empty() && m.starts_with(|c: char| c.is_lowercase()) {
                out.insert(format!("crate::{m}"));
            }
        } else if !PRIMITIFS.contains(&tete.as_str())
            && !modules.contains(&tete)
            && tete.starts_with(|c: char| c.is_lowercase())
        {
            out.insert(tete);
        }
    }
    out
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
    let connus: BTreeSet<&str> = INVENTAIRE.iter().flat_map(|l| l.fournie_par.iter().copied()).collect();
    let exemptes: BTreeSet<&str> = SANS_OBJET.iter().map(|(c, _)| *c).collect();

    let fichiers = chemin_servi(&racine);
    // Le plancher anti-vacuité. Sans lui, un dossier renommé rendrait la liste VIDE et le garde
    // serait vert sans avoir rien lu — la panne la plus confortable qu'un garde puisse avoir.
    assert!(
        fichiers.len() >= 12,
        "seulement {} fichiers de route trouvés : le dossier a-t-il déménagé ? Un garde qui ne \
         trouve rien à lire ne doit pas passer pour un garde qui n'a rien trouvé à redire",
        fichiers.len()
    );

    let mut orphelines = Vec::new();
    let mut vus = 0usize;
    for f in &fichiers {
        let rel = f.strip_prefix(&racine).unwrap_or(f).display().to_string();
        let src = std::fs::read_to_string(f)
            .unwrap_or_else(|e| panic!("{rel} illisible : {e} — le chemin servi a-t-il déménagé ?"));
        for c in dependances(&region_servie(&src)) {
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

/// **Le scénario M5, rendu vérifiable au lieu d'être affirmé.**
///
/// La doc de `capture.rs` raconte que M5 a ajouté une dépendance sur le chemin d'appel sans
/// inscrire sa ligne. Dire que ce garde l'aurait attrapée est une affirmation ; ce test en fait une
/// mesure. Il exige les deux moitiés du mécanisme :
///
/// * l'extracteur **voit** `crate::reg_pont` dans le vrai `llm.rs` — où il n'apparaît que sous forme
///   de chemin pleinement qualifié, sans aucun `use` ;
/// * l'inventaire le **connaît**. Retirez la ligne de M5 et ce test tombe, ce qui est exactement
///   l'état du dépôt le jour où l'oubli a eu lieu.
#[test]
fn le_scenario_m5_est_couvert_et_non_seulement_raconte() {
    let src = std::fs::read_to_string(racine().join("crates/efe-swarm-gateway/src/routes/llm.rs"))
        .expect("llm.rs lisible");
    let vues = dependances(&region_servie(&src));
    assert!(
        vues.contains("crate::reg_pont"),
        "l'extracteur ne voit pas la dépendance de M5 dans `llm.rs` — or elle y entre par un chemin \
         pleinement qualifié (`crate::reg_pont::RegReport`), la forme même que la première rédaction \
         de ce garde ratait"
    );
    let connus: BTreeSet<&str> = INVENTAIRE.iter().flat_map(|l| l.fournie_par.iter().copied()).collect();
    assert!(
        connus.contains("crate::reg_pont"),
        "l'inventaire ne rattache aucune ligne à `crate::reg_pont` : c'est précisément l'état dans \
         lequel M5 avait laissé le dépôt, et que ce garde existe pour rendre impossible"
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
    // Le périmètre étant propre, les gardes ci-dessus resteraient verts même si leurs extracteurs
    // étaient cassés. On les éprouve donc sur des entrées synthétiques — et notamment sur les DEUX
    // formes que la première rédaction ratait.
    let src = "//! cf. crate::oidc::Verifier\n\
               use efe_multi_llm::{A, B};\n\
               use crate::capture::decider;\n\
               fn f(x: Option<crate::reg_pont::RegReport>) -> Uuid { Uuid::new_v4() }\n\
               #[cfg(test)]\n\
               use efe_de_test::Truc;\n";
    let vus = dependances(&region_servie(src));

    assert!(vus.contains("efe_multi_llm"), "un `use` groupé doit être vu : `use foo::{{A, B}}`");
    assert!(
        vus.contains("crate::reg_pont"),
        "un chemin PLEINEMENT QUALIFIÉ doit être vu — c'est sous cette forme exacte que la \
         dépendance de M5 entre dans `llm.rs`, et c'est ce que la première rédaction ratait"
    );
    assert!(vus.contains("crate::capture"), "un `use crate::` compte pour son MODULE");
    assert!(
        !vus.contains("crate::oidc"),
        "un module cité dans un COMMENTAIRE n'est pas une dépendance : `llm.rs` documente \
         `crate::oidc` sans jamais l'appeler sur le chemin servi"
    );
    assert!(!vus.contains("efe_de_test"), "un import de test n'est pas une dépendance de service");
    assert!(!vus.contains("Uuid"), "`Uuid::new_v4()` est un appel de fonction associée, pas un module");
    assert!(!vus.contains("crate"), "le mot `crate` n'est pas une dépendance — son MODULE l'est");

    assert!(dependances("fn rien() {}").is_empty(), "et se taire quand il n'y a rien");
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
