// SPDX-License-Identifier: UNLICENSED
//! # `scella-conserver` — la conservation, **du côté de celui qui exploite**.
//!
//! ## Pourquoi ce binaire existe
//!
//! La première livraison de P2 était une bibliothèque sans un seul appelant. `grep` le disait sans
//! ambiguïté : `scella-conservation` n'apparaissait nulle part hors de sa propre arborescence et de
//! la liste des membres du workspace. Or la feuille de route ne demandait pas « une conservation
//! codée », elle demandait une conservation **opérable** — et une capacité qu'aucune commande
//! n'invoque n'est pas une opération, c'est une intention.
//!
//! Ce programme est la commande. Il ne contient aucune logique de preuve : tout ce qu'il fait, il le
//! délègue au crate, et son seul travail propre est de traduire des arguments en gestes et des
//! verdicts en codes de sortie qu'un `cron` ou un `systemd` sait lire.
//!
//! ## Les trois choses qu'il fait, que la bibliothèque ne pouvait pas faire
//!
//! 1. **Il a une horloge.** La bibliothèque n'en a pas, exprès : une horloge dans le code de preuve
//!    rend les tests non déterministes et introduit une entrée que personne ne contrôle. C'est ici
//!    que le temps entre, et `--a` permet de l'imposer quand on rejoue.
//! 2. **Il tient le trousseau des générations sur disque** — et prend donc le numéro de génération
//!    *dans le trousseau* plutôt que de la main de l'exploitant. Un manifeste ne peut plus annoncer
//!    une génération qui n'est pas celle qui a scellé.
//! 3. **Il refuse une destruction non confirmée.** Détruire une génération est irréversible et
//!    silencieux — rien ne casse, les archives concernées deviennent simplement irrécupérables des
//!    mois plus tard. Un geste dont la conséquence est différée doit demander une confirmation
//!    explicite au moment où on le pose.
//!
//! ## Codes de sortie
//!
//! | code | sens |
//! |---|---|
//! | `0` | le geste a réussi, ou l'archive se restaure |
//! | `1` | **refus motivé** : archive divergente, rotation impossible, destruction non confirmée |
//! | `2` | usage, ou entrée illisible |
//!
//! La distinction 1/2 est celle de `verify-mini` et elle compte pour un ordonnanceur : `1` veut dire
//! « j'ai regardé et ça ne va pas », `2` veut dire « je n'ai pas pu regarder ». Confondre les deux
//! ferait passer une panne de montage pour une alerte d'intégrité.

use std::path::{Path, PathBuf};
use std::process::exit;

use provenance_dag::{SealedDag, TenantKey};
use scella_conservation::archive::{empreinte_fichier, sauvegarder, Prise};
use scella_conservation::generations::EtatGeneration;
use scella_conservation::{restaurer_controle, trousseau_maitre_fichier as tmf, Controle, TrousseauMaitre};

const USAGE: &str = "\
scella-conserver — sauvegarder, restaurer, faire tourner la clé.

  sauvegarder --records <f.jsonl> --vers <dossier> --tenant <nom>
              (--trousseau-maitre <f> | --cle-hex <64hex> --generation <n> | --cle-fichier <f> --generation <n>)
              [--trousseau-cas <f>] [--ancre <run_id>] [--a <époque>]

  restaurer   --archive <dossier>
              (--trousseau-maitre <f> | --cle-hex <64hex> | --cle-fichier <f>)
              [--journal <f>] [--trousseau-cas <f>] [--vers <f.jsonl>]

  etat        --trousseau-maitre <f>
  poser       --trousseau-maitre <f> --numero <n> --cle-hex <64hex> [--a <époque>]
  tourner     --trousseau-maitre <f> --numero <n> --cle-hex <64hex> [--a <époque>]
  retirer     --trousseau-maitre <f> --numero <n> [--a <époque>]
  detruire    --trousseau-maitre <f> --numero <n> [--a <époque>] --je-confirme

Codes : 0 réussi · 1 refus motivé · 2 usage ou entrée illisible.";

fn main() {
    exit(real_main(&std::env::args().collect::<Vec<_>>()));
}

/// Le vrai point d'entrée, testable sans lancer de processus — la forme de `verify-mini`.
fn real_main(args: &[String]) -> i32 {
    let Some(commande) = args.get(1).map(String::as_str) else {
        eprintln!("{USAGE}");
        return 2;
    };
    let reste = &args[2..];
    match commande {
        "sauvegarder" => cmd_sauvegarder(reste),
        "restaurer" => cmd_restaurer(reste),
        "etat" | "état" => cmd_etat(reste),
        "poser" => cmd_generation(reste, Geste::Poser),
        "tourner" => cmd_generation(reste, Geste::Tourner),
        "retirer" => cmd_generation(reste, Geste::Retirer),
        "detruire" | "détruire" => cmd_generation(reste, Geste::Detruire),
        "-h" | "--help" | "aide" => {
            println!("{USAGE}");
            0
        }
        autre => {
            eprintln!("commande inconnue : « {autre} »\n\n{USAGE}");
            2
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════════════════
//  Analyse des arguments — à la main, parce que ce crate n'a qu'UNE dépendance et que ce
//  n'est pas une commodité d'analyse syntaxique qui va lui en faire gagner une seconde.
// ══════════════════════════════════════════════════════════════════════════════════════════

/// Les options, sous forme de paires — **confrontées à la liste des drapeaux que la commande
/// connaît**.
///
/// Cette confrontation n'est pas du zèle. Sans elle, un `--trousseau-maitr` mal orthographié serait
/// accepté et ignoré : la sauvegarde partirait sans trousseau, le manifeste porterait une génération
/// venue d'ailleurs, et la commande sortirait **0**. Un défaut silencieux dans l'outil dont tout
/// l'objet est de ne pas perdre de preuve.
///
/// (Le premier jet de ce fichier promettait ce contrôle dans ce commentaire même et ne le faisait
/// pas. C'est le test qui l'a attrapé, pas la relecture.)
type Drapeau = (String, Option<String>);
/// Les options d'une commande, dans l'ordre où elles ont été données.
type Options = Vec<Drapeau>;

fn options(args: &[String], connus: &[&str]) -> Result<Options, String> {
    const SANS_VALEUR: [&str; 1] = ["--je-confirme"];
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if !a.starts_with("--") {
            return Err(format!("argument inattendu : « {a} »"));
        }
        if !connus.contains(&a.as_str()) {
            return Err(format!(
                "drapeau inconnu : « {a} ». Connus ici : {}",
                connus.join(", ")
            ));
        }
        if out.iter().any(|(k, _): &Drapeau| k == a) {
            return Err(format!("« {a} » est donné deux fois — lequel serait le bon ?"));
        }
        if SANS_VALEUR.contains(&a.as_str()) {
            out.push((a.clone(), None));
            i += 1;
            continue;
        }
        let v = args.get(i + 1).ok_or_else(|| format!("« {a} » attend une valeur"))?;
        if v.starts_with("--") {
            return Err(format!("« {a} » attend une valeur, pas un autre drapeau"));
        }
        out.push((a.clone(), Some(v.clone())));
        i += 2;
    }
    Ok(out)
}

fn val<'a>(o: &'a [Drapeau], nom: &str) -> Option<&'a str> {
    o.iter().find(|(k, _)| k == nom).and_then(|(_, v)| v.as_deref())
}

fn present(o: &[Drapeau], nom: &str) -> bool {
    o.iter().any(|(k, _)| k == nom)
}

fn requis<'a>(o: &'a [Drapeau], nom: &str) -> Result<&'a str, String> {
    val(o, nom).ok_or_else(|| format!("« {nom} » est obligatoire"))
}

/// L'instant courant, ou celui que l'exploitant impose avec `--a` (rejeu, tests, reprise).
fn maintenant(o: &[Drapeau]) -> Result<u64, String> {
    match val(o, "--a") {
        Some(v) => v.parse::<u64>().map_err(|_| format!("« --a » : « {v} » n'est pas une époque")),
        None => Ok(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)),
    }
}

fn hex_vers_octets(hex: &str) -> Result<[u8; 32], String> {
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("une clé fait 64 caractères hexadécimaux".into());
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// D'où vient la clé — et le numéro de génération qui va avec, quand la source le sait.
///
/// `--trousseau-maitre` est la voie recommandée, et pas seulement par confort : le numéro de
/// génération vient alors du **trousseau**, pas de la mémoire de l'exploitant. Un manifeste ne peut
/// donc plus annoncer une génération qui n'est pas celle qui a scellé — la divergence que
/// `Issue::MauvaiseGeneration` existe pour rattraper devient impossible à créer par inadvertance.
/// D'où vient la clé — et ce que la source sait dire de plus.
///
/// Trois champs nommés plutôt qu'un triplet : `generation` et `trousseau` sont tous deux
/// `Option`, et un `(_, None, Some(_))` lu à la volée n'apprend rien à personne.
struct SourceCle {
    /// La clé qui scelle ou re-prouve.
    cle: TenantKey,
    /// Le numéro de génération, quand la source le connaît — le trousseau le connaît toujours.
    generation: Option<u32>,
    /// Le trousseau complet, quand la clé en vient. Permet à la restauration de PROUVER une
    /// mauvaise génération au lieu de la supposer.
    trousseau: Option<TrousseauMaitre>,
}

fn cle_et_generation(o: &[Drapeau], exiger_generation: bool) -> Result<SourceCle, String> {
    let sources = ["--trousseau-maitre", "--cle-hex", "--cle-fichier"]
        .iter()
        .filter(|s| present(o, s))
        .count();
    if sources != 1 {
        return Err("fournir EXACTEMENT une source de clé : --trousseau-maitre, --cle-hex ou --cle-fichier".into());
    }

    if let Some(chemin) = val(o, "--trousseau-maitre") {
        let (t, trop_ouvert) = tmf::charger(Path::new(chemin)).map_err(|e| e.to_string())?;
        if trop_ouvert {
            eprintln!("⚠ « {chemin} » est lisible par d'autres que son propriétaire — il porte des clés.");
        }
        let (numero, cle) = t
            .courante()
            .map(|(n, c)| (n, c.clone()))
            .ok_or("ce trousseau n'a aucune génération vivante : rien ne peut sceller")?;
        return Ok(SourceCle { cle, generation: Some(numero), trousseau: Some(t) });
    }

    let hex = if let Some(h) = val(o, "--cle-hex") {
        eprintln!("⚠ --cle-hex laisse la clé dans l'historique du shell et dans `ps`. Préférez --cle-fichier.");
        h.to_string()
    } else {
        let chemin = val(o, "--cle-fichier").expect("compté juste au-dessus");
        std::fs::read_to_string(chemin)
            .map_err(|e| format!("« {chemin} » illisible : {e}"))?
            .trim()
            .to_string()
    };
    let cle = TenantKey::from_bytes(hex_vers_octets(&hex)?);

    let generation = match val(o, "--generation") {
        Some(g) => Some(g.parse::<u32>().map_err(|_| format!("« --generation » : « {g} » n'est pas un nombre"))?),
        None if exiger_generation => {
            return Err("« --generation » est obligatoire avec --cle-hex/--cle-fichier (le trousseau le saurait)".into())
        }
        None => None,
    };
    Ok(SourceCle { cle, generation, trousseau: None })
}

// ══════════════════════════════════════════════════════════════════════════════════════════
//  Les commandes
// ══════════════════════════════════════════════════════════════════════════════════════════

/// Les drapeaux que `sauvegarder` connaît.
const F_SAUVEGARDER: &[&str] = &[
    "--records", "--vers", "--tenant", "--trousseau-maitre", "--cle-hex", "--cle-fichier",
    "--generation", "--trousseau-cas", "--ancre", "--a",
];
/// Ceux de `restaurer`.
const F_RESTAURER: &[&str] = &[
    "--archive", "--trousseau-maitre", "--cle-hex", "--cle-fichier", "--generation", "--journal",
    "--trousseau-cas", "--vers",
];
/// Ceux de `etat`.
const F_ETAT: &[&str] = &["--trousseau-maitre"];
/// Ceux des gestes de génération. `--cle-hex` n'a de sens que pour `poser`/`tourner`, et
/// `--je-confirme` que pour `detruire` : les accepter partout serait un mensonge bénin, mais un
/// mensonge — un `--je-confirme` sur un `retirer` laisserait croire qu'il y avait quelque chose à
/// confirmer.
const F_POSER: &[&str] = &["--trousseau-maitre", "--numero", "--cle-hex", "--a"];
const F_RETIRER: &[&str] = &["--trousseau-maitre", "--numero", "--a"];
const F_DETRUIRE: &[&str] = &["--trousseau-maitre", "--numero", "--a", "--je-confirme"];

fn cmd_sauvegarder(args: &[String]) -> i32 {
    let o = match options(args, F_SAUVEGARDER) {
        Ok(o) => o,
        Err(e) => return usage(&e),
    };
    let (records, vers, tenant) = match (requis(&o, "--records"), requis(&o, "--vers"), requis(&o, "--tenant")) {
        (Ok(a), Ok(b), Ok(c)) => (a, b, c),
        (a, b, c) => return usage(&[a.err(), b.err(), c.err()].into_iter().flatten().collect::<Vec<_>>().join(" ; ")),
    };
    let source = match cle_et_generation(&o, true) {
        Ok(x) => x,
        Err(e) => return usage(&e),
    };
    let cree_a = match maintenant(&o) {
        Ok(t) => t,
        Err(e) => return usage(&e),
    };

    let brut = match std::fs::read_to_string(records) {
        Ok(b) => b,
        Err(e) => return usage(&format!("« {records} » illisible : {e}")),
    };
    let recs = match SealedDag::records_from_jsonl(&brut) {
        Ok(r) => r,
        Err(e) => return usage(&format!("« {records} » n'est pas un JSONL de records : {e:?}")),
    };

    let empreinte_trousseau = match val(&o, "--trousseau-cas") {
        None => None,
        Some(c) => match empreinte_fichier(Path::new(c)) {
            Ok(e) => Some(e),
            Err(e) => return usage(&format!("« {c} » illisible : {e}")),
        },
    };

    let prise = Prise {
        tenant: tenant.to_string(),
        cree_a,
        generation: source.generation.expect("exigée juste au-dessus"),
        empreinte_trousseau,
        ancre: val(&o, "--ancre").map(str::to_string),
    };

    // Le verrou : deux sauvegardes concurrentes vers le MÊME dossier écrivent records.jsonl et
    // MANIFESTE indépendamment. Chacune est atomique, la PAIRE ne l'est pas — on pourrait obtenir
    // les records de l'une et le manifeste de l'autre, c'est-à-dire une archive cohérente en
    // apparence et fausse en fait. Le verrou porte sur le dossier, pas sur les fichiers.
    let _verrou = match Verrou::prendre(Path::new(vers)) {
        Ok(v) => v,
        Err(e) => return refus(&e),
    };

    match sauvegarder(&recs, &source.cle, &prise, Path::new(vers)) {
        Ok(s) => {
            println!("archivé : {} records, racine {}", s.manifeste.noeuds, s.manifeste.racine);
            println!("  génération {} · tenant {}", s.manifeste.generation, s.manifeste.tenant);
            println!("  → {}", s.dossier.display());
            0
        }
        Err(e) => refus(&e.to_string()),
    }
}

fn cmd_restaurer(args: &[String]) -> i32 {
    let o = match options(args, F_RESTAURER) {
        Ok(o) => o,
        Err(e) => return usage(&e),
    };
    let archive = match requis(&o, "--archive") {
        Ok(a) => a,
        Err(e) => return usage(&e),
    };
    let source = match cle_et_generation(&o, false) {
        Ok(x) => x,
        Err(e) => return usage(&e),
    };

    // Sans trousseau maître, on en fabrique un d'une seule génération : le diagnostic reste le même
    // chemin de code, et le message « mauvaise génération » ne peut pas mentir faute de comparant.
    //
    // Le NUMÉRO de cette génération se lit dans le manifeste, jamais dans un défaut à zéro. Un
    // exploitant qui présente la bonne clé n'a pas à deviner un numéro qui est écrit dans l'archive
    // qu'il tient : le faire répondrait « génération inconnue » à quelqu'un dont la clé est juste,
    // et masquerait le vrai verdict — c'est exactement ce que le contrôle de l'ancre a révélé.
    let trousseau = source.trousseau.unwrap_or_else(|| {
        let generation = source.generation.or_else(|| generation_annoncee(Path::new(archive))).unwrap_or(0);
        let mut t = TrousseauMaitre::new();
        let _ = t.poser(generation, source.cle.clone(), 0);
        t
    });

    let journal = match val(&o, "--journal") {
        None => None,
        Some(c) => match std::fs::read_to_string(c) {
            Ok(j) => Some(j),
            Err(e) => return usage(&format!("journal « {c} » illisible : {e}")),
        },
    };
    let trousseau_cas = val(&o, "--trousseau-cas").map(PathBuf::from);
    let controle = Controle {
        journal_ancres: journal.as_deref(),
        trousseau_cas: trousseau_cas.as_deref(),
    };

    let r = restaurer_controle(Path::new(archive), &trousseau, &controle);
    if let Some(m) = &r.manifeste {
        println!("archive : tenant {} · génération {} · {} nœuds annoncés", m.tenant, m.generation, m.noeuds);
    }
    if !r.issue.est_restauree() {
        eprintln!("✗ {}", r.issue);
        return 1;
    }
    println!("✓ {}", r.issue);
    if journal.is_none() {
        println!("  (aucune ancre confrontée : cohérence de l'archive AVEC ELLE-MÊME seulement)");
    }
    if let Some(sortie) = val(&o, "--vers") {
        let mut jsonl = SealedDag::from_records(r.records.clone()).to_jsonl();
        jsonl.push('\n');
        if let Err(e) = std::fs::write(sortie, jsonl) {
            eprintln!("restaurée, mais « {sortie} » non écrit : {e}");
            return 2;
        }
        println!("  → {sortie}");
    }
    0
}

/// La génération que le manifeste d'une archive annonce, si on peut la lire.
///
/// Rend `None` en silence : ce n'est pas ici qu'on diagnostique une archive illisible. Le
/// diagnostic complet appartient à `restaurer_controle`, qui dira *pourquoi* le manifeste ne se lit
/// pas — dupliquer ce refus ici produirait deux messages pour une seule cause.
fn generation_annoncee(archive: &Path) -> Option<u32> {
    let texte = std::fs::read_to_string(archive.join("MANIFESTE")).ok()?;
    scella_conservation::manifeste::Manifeste::lire(&texte).ok().map(|m| m.generation)
}

fn cmd_etat(args: &[String]) -> i32 {
    let o = match options(args, F_ETAT) {
        Ok(o) => o,
        Err(e) => return usage(&e),
    };
    let chemin = match requis(&o, "--trousseau-maitre") {
        Ok(c) => c,
        Err(e) => return usage(&e),
    };
    let (t, trop_ouvert) = match tmf::charger(Path::new(chemin)) {
        Ok(x) => x,
        Err(e) => return usage(&e.to_string()),
    };
    if trop_ouvert {
        eprintln!("⚠ « {chemin} » est lisible par d'autres que son propriétaire — il porte des clés.");
    }
    let plus_haut = t.plus_haut_connu().unwrap_or(0);
    println!("génération  état       scelle  re-prouve");
    for n in 1..=plus_haut {
        let (etat, scelle, reprouve) = match t.etat(n) {
            EtatGeneration::Vivante { depuis } => (format!("VIVANTE (depuis {depuis})"), "oui", "oui"),
            EtatGeneration::Retiree { le } => (format!("RETIREE (le {le})"), "non", "oui"),
            EtatGeneration::Detruite { le } => (format!("DÉTRUITE (le {le})"), "non", "NON"),
            EtatGeneration::Inconnue => continue,
        };
        println!("{n:>10}  {etat:<26} {scelle:<6}  {reprouve}");
    }
    match t.courante() {
        Some((n, _)) => println!("\nc'est la génération {n} qui scelle le neuf."),
        None => println!("\nAUCUNE génération vivante : ce trousseau ne peut rien sceller."),
    }
    0
}

enum Geste {
    Poser,
    Tourner,
    Retirer,
    Detruire,
}

fn cmd_generation(args: &[String], geste: Geste) -> i32 {
    let connus = match geste {
        Geste::Poser | Geste::Tourner => F_POSER,
        Geste::Retirer => F_RETIRER,
        Geste::Detruire => F_DETRUIRE,
    };
    let o = match options(args, connus) {
        Ok(o) => o,
        Err(e) => return usage(&e),
    };
    let (chemin, numero) = match (requis(&o, "--trousseau-maitre"), requis(&o, "--numero")) {
        (Ok(c), Ok(n)) => match n.parse::<u32>() {
            Ok(n) => (c, n),
            Err(_) => return usage(&format!("« --numero » : « {n} » n'est pas un nombre")),
        },
        (a, b) => return usage(&[a.err(), b.err()].into_iter().flatten().collect::<Vec<_>>().join(" ; ")),
    };
    let quand = match maintenant(&o) {
        Ok(t) => t,
        Err(e) => return usage(&e),
    };
    let p = Path::new(chemin);

    let r = match geste {
        Geste::Poser | Geste::Tourner => {
            let hex = match requis(&o, "--cle-hex") {
                Ok(h) => h,
                Err(e) => return usage(&e),
            };
            eprintln!("⚠ --cle-hex laisse la clé dans l'historique du shell et dans `ps`.");
            if matches!(geste, Geste::Poser) {
                tmf::poser_generation(p, numero, hex, quand)
            } else {
                tmf::tourner(p, numero, hex, quand)
            }
        }
        Geste::Retirer => tmf::retirer_generation(p, numero, quand),
        Geste::Detruire => {
            // Irréversible, et sa conséquence est DIFFÉRÉE : rien ne casse aujourd'hui, les
            // archives de cette génération deviennent irrécupérables des mois plus tard. Un geste
            // dont le dommage n'apparaît pas tout de suite doit se confirmer au moment où on le pose.
            if !present(&o, "--je-confirme") {
                eprintln!(
                    "REFUSÉ : détruire la génération {numero} est IRRÉVERSIBLE. Toute archive scellée \n\
                     sous elle deviendra définitivement irrécupérable — pas aujourd'hui, le jour où \n\
                     quelqu'un tentera de la restaurer. Ajoutez --je-confirme."
                );
                return 1;
            }
            tmf::detruire_generation(p, numero, quand)
        }
    };

    match r {
        Ok(()) => {
            let quoi = match geste {
                Geste::Poser => "posée",
                Geste::Tourner => "en service (la précédente est retirée, elle re-prouve encore)",
                Geste::Retirer => "retirée (elle ne scelle plus, elle re-prouve encore)",
                Geste::Detruire => "DÉTRUITE (sa clé a quitté le fichier)",
            };
            println!("génération {numero} : {quoi}, à {quand}.");
            0
        }
        Err(e) => refus(&e.to_string()),
    }
}

// ══════════════════════════════════════════════════════════════════════════════════════════
//  Le verrou de dossier — atomique, sans dépendance
// ══════════════════════════════════════════════════════════════════════════════════════════

/// Un verrou consultatif sur un dossier d'archive, tenu par un fichier créé en `O_EXCL`.
///
/// `create_new(true)` est atomique : deux processus qui le tentent en même temps, un seul réussit.
/// Aucune bibliothèque n'est nécessaire, ce qui garde la fermeture de ce crate à une dépendance.
///
/// Le verrou porte le PID et l'instant, pour qu'un exploitant qui tombe sur un verrou resté après un
/// arrêt brutal sache **qui** le tenait et **depuis quand** — un verrou anonyme ne laisse d'autre
/// choix que de le supprimer à l'aveugle, ce qui revient à n'avoir pas de verrou.
#[derive(Debug)]
struct Verrou(PathBuf);

impl Verrou {
    fn prendre(dossier: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dossier).map_err(|e| format!("dossier « {} » non créé : {e}", dossier.display()))?;
        let chemin = dossier.join(".conservation.verrou");
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&chemin) {
            Ok(mut f) => {
                use std::io::Write;
                let quand = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let _ = writeln!(f, "pid={} depuis={}", std::process::id(), quand);
                Ok(Verrou(chemin))
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let qui = std::fs::read_to_string(&chemin).unwrap_or_default();
                Err(format!(
                    "une autre sauvegarde tient « {} » ({}). Si elle est morte, supprimez {}.",
                    dossier.display(),
                    qui.trim(),
                    chemin.display()
                ))
            }
            Err(e) => Err(format!("verrou impossible : {e}")),
        }
    }
}

impl Drop for Verrou {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn usage(motif: &str) -> i32 {
    eprintln!("{motif}\n\n{USAGE}");
    2
}

fn refus(motif: &str) -> i32 {
    eprintln!("✗ {motif}");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dossier(nom: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("scella-cli-{}-{}", nom, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn sans_commande_c_est_un_usage_pas_un_refus() {
        assert_eq!(real_main(&s(&["prog"])), 2);
    }

    #[test]
    fn une_commande_inconnue_sort_2() {
        assert_eq!(real_main(&s(&["prog", "sauvgarder"])), 2, "la faute de frappe ne doit RIEN faire");
    }

    #[test]
    fn un_drapeau_inconnu_est_refuse_au_lieu_d_etre_ignore() {
        // Un `--trousseau-maitr` ignoré ferait sauvegarder sans trousseau, en silence, code 0.
        let e = options(&s(&["--inconnu", "x"]), F_SAUVEGARDER).unwrap_err();
        assert!(e.contains("inconnu") && e.contains("--inconnu"), "{e}");
    }

    #[test]
    fn un_drapeau_qui_avale_le_suivant_est_refuse() {
        let e = options(&s(&["--vers", "--tenant"]), F_SAUVEGARDER).unwrap_err();
        assert!(e.contains("pas un autre drapeau"), "{e}");
    }

    #[test]
    fn deux_sources_de_cle_sont_refusees() {
        let o = options(&s(&["--cle-hex", &"aa".repeat(32), "--cle-fichier", "/x"]), F_SAUVEGARDER).unwrap();
        // `unwrap_err` exigerait `Debug` sur `SourceCle`, qui porte une clé. On ne donne pas une
        // surface d'impression à une structure qui tient un secret pour arranger un test.
        let Err(e) = cle_et_generation(&o, false) else {
            panic!("deux sources de clé doivent être refusées")
        };
        assert!(e.contains("EXACTEMENT une"), "{e}");
    }

    #[test]
    fn detruire_sans_confirmation_sort_1_et_ne_touche_a_rien() {
        let d = dossier("detruire");
        let f = d.join("tm");
        tmf::poser_generation(&f, 1, &"01".repeat(32), 100).unwrap();
        let avant = std::fs::read_to_string(&f).unwrap();
        let code = real_main(&s(&["prog", "detruire", "--trousseau-maitre", f.to_str().unwrap(), "--numero", "1"]));
        assert_eq!(code, 1, "une destruction non confirmée est un REFUS, pas une erreur d'usage");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), avant, "le fichier est intact");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn le_verrou_empeche_deux_sauvegardes_simultanees() {
        let d = dossier("verrou");
        let a = Verrou::prendre(&d).expect("le premier prend");
        let e = Verrou::prendre(&d).expect_err("le second doit échouer");
        assert!(e.contains("pid="), "le refus doit dire QUI tient le verrou : {e}");
        drop(a);
        Verrou::prendre(&d).expect("libéré, il se reprend");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn le_cycle_complet_passe_par_la_ligne_de_commande() {
        let d = dossier("cycle");
        let tm = d.join("generations");
        let recs = d.join("records.jsonl");
        let arch = d.join("archive");

        // Un DAG minimal à archiver.
        let cle_hex = "2a".repeat(32);
        let cle = TenantKey::from_bytes(hex_vers_octets(&cle_hex).unwrap());
        let mut dag = SealedDag::new();
        dag.append(
            &cle,
            provenance_dag::Node::AgentAct {
                node_id: "CAS::a".into(),
                agent_role: "r".into(),
                inputs_hash: "h".into(),
                model: "m".into(),
                version: "1".into(),
                output: "PASS".into(),
                evidence_refs: vec![],
                timestamp: 1,
            },
        )
        .unwrap();
        std::fs::write(&recs, format!("{}\n", dag.to_jsonl())).unwrap();

        // poser → sauvegarder → restaurer, sans jamais nommer la génération à la main.
        assert_eq!(
            real_main(&s(&["p", "poser", "--trousseau-maitre", tm.to_str().unwrap(), "--numero", "1", "--cle-hex", &cle_hex, "--a", "100"])),
            0
        );
        assert_eq!(
            real_main(&s(&["p", "sauvegarder", "--records", recs.to_str().unwrap(), "--vers", arch.to_str().unwrap(), "--tenant", "acme", "--trousseau-maitre", tm.to_str().unwrap(), "--a", "200"])),
            0
        );
        assert_eq!(
            real_main(&s(&["p", "restaurer", "--archive", arch.to_str().unwrap(), "--trousseau-maitre", tm.to_str().unwrap()])),
            0
        );

        // Le manifeste porte la génération que le TROUSSEAU connaissait, pas une saisie humaine.
        let m = std::fs::read_to_string(arch.join("MANIFESTE")).unwrap();
        assert!(m.contains("generation\t1"), "{m}");

        // Après rotation, l'archive ancienne se restaure encore.
        assert_eq!(
            real_main(&s(&["p", "tourner", "--trousseau-maitre", tm.to_str().unwrap(), "--numero", "2", "--cle-hex", &"07".repeat(32), "--a", "300"])),
            0
        );
        assert_eq!(
            real_main(&s(&["p", "restaurer", "--archive", arch.to_str().unwrap(), "--trousseau-maitre", tm.to_str().unwrap()])),
            0,
            "après rotation, l'ancien doit encore remonter"
        );

        // Détruite (confirmée), elle ne remonte plus — et le code est 1, pas 2.
        assert_eq!(
            real_main(&s(&["p", "detruire", "--trousseau-maitre", tm.to_str().unwrap(), "--numero", "1", "--a", "400", "--je-confirme"])),
            0
        );
        assert_eq!(
            real_main(&s(&["p", "restaurer", "--archive", arch.to_str().unwrap(), "--trousseau-maitre", tm.to_str().unwrap()])),
            1,
            "une génération détruite est un REFUS MOTIVÉ (1), pas une entrée illisible (2)"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod tests_liste_blanche {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn une_faute_de_frappe_sur_le_trousseau_est_fatale() {
        // Le cas concret : sans liste blanche, la sauvegarde serait partie SANS trousseau, avec un
        // manifeste portant une génération venue d'ailleurs, et un code de sortie 0.
        let e = options(&s(&["--trousseau-maitr", "/x"]), F_SAUVEGARDER).unwrap_err();
        assert!(e.contains("drapeau inconnu"), "{e}");
    }

    #[test]
    fn je_confirme_n_est_accepte_que_par_detruire() {
        assert!(options(&s(&["--je-confirme"]), F_DETRUIRE).is_ok());
        assert!(
            options(&s(&["--je-confirme"]), F_RETIRER).is_err(),
            "confirmer un retrait laisserait croire qu'il y avait quelque chose à confirmer"
        );
    }

    #[test]
    fn un_drapeau_donne_deux_fois_est_refuse_au_lieu_d_en_choisir_un() {
        let e = options(&s(&["--tenant", "a", "--tenant", "b"]), F_SAUVEGARDER).unwrap_err();
        assert!(e.contains("deux fois"), "{e}");
    }

    #[test]
    fn le_message_liste_ce_qui_est_accepte() {
        let e = options(&s(&["--journal", "/j"]), F_ETAT).unwrap_err();
        assert!(e.contains("--trousseau-maitre"), "un refus doit dire quoi faire : {e}");
    }
}

#[cfg(test)]
mod tests_generation_lue {
    use super::*;

    #[test]
    fn une_cle_nue_n_a_pas_a_deviner_le_numero_de_generation() {
        // Le piège attrapé par le contrôle de l'ancre : sans cette lecture, une clé PARFAITEMENT
        // juste recevait « génération 1 inconnue » parce que le binaire avait supposé 0 — et ce
        // faux refus masquait le vrai verdict qu'on cherchait à obtenir.
        let d = std::env::temp_dir().join(format!("scella-gen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let arch = d.join("a");
        let cle_hex = "2a".repeat(32);
        let cle = TenantKey::from_bytes(hex_vers_octets(&cle_hex).unwrap());
        let mut dag = SealedDag::new();
        dag.append(
            &cle,
            provenance_dag::Node::AgentAct {
                node_id: "CAS::a".into(),
                agent_role: "r".into(),
                inputs_hash: "h".into(),
                model: "m".into(),
                version: "1".into(),
                output: "PASS".into(),
                evidence_refs: vec![],
                timestamp: 1,
            },
        )
        .unwrap();
        let recs = d.join("r.jsonl");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(&recs, format!("{}\n", dag.to_jsonl())).unwrap();

        // Archivée en annonçant la génération 9 — un numéro que rien ne laisse deviner.
        let s_ = |v: &[&str]| -> Vec<String> { v.iter().map(|x| (*x).to_string()).collect() };
        assert_eq!(
            real_main(&s_(&["p", "sauvegarder", "--records", recs.to_str().unwrap(), "--vers", arch.to_str().unwrap(), "--tenant", "t", "--cle-hex", &cle_hex, "--generation", "9", "--a", "1"])),
            0
        );
        assert_eq!(generation_annoncee(&arch), Some(9), "le manifeste porte bien 9");
        assert_eq!(
            real_main(&s_(&["p", "restaurer", "--archive", arch.to_str().unwrap(), "--cle-hex", &cle_hex])),
            0,
            "la bonne clé doit suffire : le numéro est écrit dans l'archive"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
