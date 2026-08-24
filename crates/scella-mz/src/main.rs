// SPDX-License-Identifier: UNLICENSED
//! # `scella-mz` — le balayage.
//!
//! Interroge le **vrai** compresseur (`CompressorClient`, llama-server, `compressor.gguf`) sur le
//! corpus figé, à cibles de compression décroissantes, et rend le verdict.
//!
//! ## Le code de sortie ne juge PAS le Cadrage
//!
//! MZ existe pour pouvoir dire non. Si un verdict « TUÉ » faisait sortir en erreur, alors MZ
//! **réussissant** ferait rougir une CI — et la pression serait d'assouplir les seuils jusqu'à ce
//! que le vert revienne. C'est exactement le mécanisme qui rend un garde décoratif.
//!
//! Donc : **sortie 0 si la mesure a eu lieu**, quel que soit le verdict. Sortie non nulle seulement
//! quand on n'a **pas pu mesurer** — serveur injoignable, modèle absent. Le verdict, lui, est écrit
//! en toutes lettres dans le rapport et sur la sortie d'erreur.
//!
//! ## Usage
//!
//! ```text
//! llama-server -m models/compressor.gguf --port 8081 &
//! cargo run -p scella-mz
//! ```
//!
//! `EFE_COMPRESSOR_URL` change l'endpoint · `EFE_COMPRESSOR_TEMPERATURE` rejoue le comportement
//! pré-correctif H1.1 (0.2, non déterministe, ≈ 33 % de dégénérescence mesurés à l'époque).
//!
//! ## `--rendre` : le rapport est une fonction des mesures
//!
//! ```text
//! cargo run -p scella-mz -- --rendre
//! ```
//!
//! Re-fabrique le `.md` **depuis le `.json`**, sans un seul appel au modèle. Le rendu ne *recopie*
//! pas les paliers ni le verdict enregistrés : il les **recalcule** depuis les points bruts, puis
//! les compare à ce qui était enregistré. Un écart signale que le rapport ne dérive plus des
//! mesures — un artefact qui aurait dérivé de sa source, exactement le défaut que Scella traque.
//!
//! C'est aussi la seule manière honnête de corriger une **formulation** du rapport : re-lancer le
//! balayage changerait les chiffres, donc l'artefact ne prouverait plus rien du changement de mots.

use std::sync::Arc;
use std::time::Instant;

use efe_nano_app_coordinator::types::{CompressionStyle, CompressorInput};
use efe_nano_app_coordinator::clients::llama_server::LlamaServerClient;
use efe_nano_app_coordinator::{
    clients::compressor::CompressorClient, CallContext, InMemoryAuditWriter, NanoRobotClient,
};
use scella_mz::corpus::{Famille, CORPUS};
use scella_mz::mesure::{paliers_du_bras, verdict, Bras, Point};
use scella_mz::verifieur::{taux_declare, taux_verifie};

/// Les cibles balayées, du généreux à l'agressif. Un document de ~330 mots donne ainsi des ratios
/// d'environ 1,7:1 à 16:1 — la fourchette revendiquée (8:1 à 12:1) est **encadrée**, ce qui est la
/// seule façon de savoir de quel côté le plafond tombe.
const CIBLES: &[u32] = &[200, 120, 80, 50, 30, 20];

/// Le document sur lequel on éprouve le déterminisme, par famille : deux appels identiques.
const CIBLE_DETERMINISME: u32 = 50;

/// Le délai d'un appel. Le défaut du client est **5 s**, et sa propre documentation dit que c'est
/// « le défaut historique », insuffisant « dès que `max_tokens` dépasse ce qu'un petit modèle génère
/// en ~5 s ». La v1 y a perdu 5 mesures sur 36 — un artefact de mesure, pas un fait sur le
/// compresseur. On le relève.
const DELAI: std::time::Duration = std::time::Duration::from_secs(60);

/// La température par défaut du compresseur, **reproduite** ici parce que `compressor_temperature()`
/// est privée. Un test fige l'accord : si le client change de défaut sans que MZ suive, les deux
/// mesureraient deux choses différentes en le disant pareil.
const TEMPERATURE_DEFAUT: f32 = 0.0;

/// Les deux bras. `Exige` éprouve la promesse du contrat ; `Servi` éprouve **ce que le pipeline
/// obtient réellement** — il passe `must_preserve: vec![]` aux deux sites de production.
const BRAS: &[Bras] = &[Bras::Exige, Bras::Servi];

#[tokio::main]
async fn main() {
    if std::env::args().any(|a| a == "--rendre") {
        std::process::exit(rendre_depuis_le_json());
    }

    let endpoint =
        std::env::var("EFE_COMPRESSOR_URL").unwrap_or_else(|_| "http://127.0.0.1:8081".to_string());

    eprintln!("┌─ MZ · la mesure du Cadrage ────────────────────────────────────────────────");
    eprintln!("│ compresseur : {endpoint}");
    eprintln!(
        "│ température : {} {}",
        std::env::var("EFE_COMPRESSOR_TEMPERATURE").unwrap_or_else(|_| "0.0 (défaut)".into()),
        if std::env::var("EFE_COMPRESSOR_TEMPERATURE").is_ok() { "⟵ override A/B" } else { "" }
    );
    eprintln!("│ corpus      : {} documents, {} cibles", CORPUS.len(), CIBLES.len());
    eprintln!("└───────────────────────────────────────────────────────────────────────────\n");

    // On reconstruit le client à la main plutôt que par `from_assets` : c'est le seul moyen de
    // relever le délai (`with_timeouts`), et la v1 a perdu 5 mesures sur 36 faute de l'avoir fait.
    let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../efe-nano-app-coordinator/assets");
    let temperature: f32 = std::env::var("EFE_COMPRESSOR_TEMPERATURE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(TEMPERATURE_DEFAUT);
    let inner = LlamaServerClient::from_assets(
        "compressor",
        "1.0.0",
        &endpoint,
        assets.join("grammars/compressor_v1.gbnf"),
        assets.join("prompts/compressor.txt"),
        1536,
        temperature,
    )
    .and_then(|c| c.with_timeouts(DELAI));
    let client = match inner {
        Ok(c) => CompressorClient::from_helper(c),
        Err(e) => {
            eprintln!("MESURE IMPOSSIBLE — le client ne se construit pas : {e}");
            eprintln!("  (assets manquants ? cf. crates/efe-nano-app-coordinator/assets/)");
            std::process::exit(2);
        }
    };
    // Une mesure n'a pas de tenant. On en fige un plutôt que d'en tirer un au hasard : deux
    // exécutions de MZ doivent différer par ce qu'elles mesurent, pas par leur contexte.
    let ctx = CallContext::new(uuid::Uuid::nil(), Arc::new(InMemoryAuditWriter::default()));

    let mut points: Vec<Point> = Vec::new();
    let mut echecs = 0usize;

    for bras in BRAS {
    for cible in CIBLES {
        for doc in CORPUS {
            // ⭐ La correction majeure de la v2. Le bras `Exige` demande la préservation — ce que
            //    le pipeline ne fait JAMAIS. Le bras `Servi` reproduit sa configuration réelle
            //    (`must_preserve: vec![]`), où le filtre du client devient vacant et où la
            //    vérification indépendante de MZ est le seul contrôle qui subsiste.
            //
            //    Mesurer `Exige` seul, comme la v1, avait deux défauts : on décrivait une
            //    configuration que personne n'exécute, et le filtre écartait au passage les
            //    réponses où le modèle AVOUE une omission — ne laissant passer que celles
            //    qui déclarent tout avoir gardé.
            let exigees: Vec<String> = match bras {
                Bras::Exige => doc.entites.iter().map(|e| (*e).to_string()).collect(),
                Bras::Servi => Vec::new(),
            };
            let entree = CompressorInput {
                text: doc.texte.to_string(),
                target_words: *cible,
                must_preserve: exigees,
                style: CompressionStyle::Extractive,
            };

            let t0 = Instant::now();
            let sortie = client.invoke(entree, &ctx).await;
            let ms = t0.elapsed().as_millis() as u64;

            match sortie {
                Ok(o) => {
                    let declare = taux_declare(&o.preserved, doc.entites);
                    let verifie = taux_verifie(&o.summary, doc.entites);
                    let mut p = Point::nouveau(
                        *bras,
                        doc.id,
                        doc.famille,
                        *cible,
                        doc.mots(),
                        &o.summary,
                        &declare,
                        &verifie,
                        ms,
                    );

                    // Déterminisme : un second appel identique, sur un document par famille.
                    if *cible == CIBLE_DETERMINISME && premier_de_sa_famille(doc.id, doc.famille) {
                        let bis = CompressorInput {
                            text: doc.texte.to_string(),
                            target_words: *cible,
                            must_preserve: match bras {
                                Bras::Exige => doc.entites.iter().map(|e| (*e).to_string()).collect(),
                                Bras::Servi => Vec::new(),
                            },
                            style: CompressionStyle::Extractive,
                        };
                        p.stable = client.invoke(bis, &ctx).await.ok().map(|b| b.summary == o.summary);
                    }

                    eprintln!(
                        "  [{}] {:<10} cible {:>3}  ratio {:>5.1}:1  déclaré {:>5.1} %  vérifié {:>5.1} % {} {}",
                        bras.code(),
                        doc.id,
                        cible,
                        p.ratio,
                        p.preservation_declaree * 100.0,
                        p.preservation_verifiee * 100.0,
                        if p.declaration_infondee() { "⚠ INFONDÉE" } else { "" },
                        if p.degenere { "⚠ DÉGÉNÉRÉ" } else { "" }
                    );
                    points.push(p);
                }
                Err(e) => {
                    echecs += 1;
                    eprintln!("  [{}] {:<10} cible {:>3}  ÉCHEC : {e}", bras.code(), doc.id, cible);
                }
            }
        }
    }
    }

    if points.is_empty() {
        eprintln!("\nMESURE IMPOSSIBLE — aucun appel n'a abouti ({echecs} échec(s)).");
        eprintln!("  Le serveur tourne-t-il ?  llama-server -m models/compressor.gguf --port 8081");
        std::process::exit(2);
    }

    // Le verdict porte sur `Exige` : c'est là que la PROMESSE du contrat est éprouvée. Le bras
    // `Servi` décrit ce que le pipeline obtient, et il figure au rapport — mais juger le compresseur
    // sur une configuration où on ne lui demande rien serait l'acquitter par construction.
    let table = paliers_du_bras(&points, Bras::Exige);
    let servi = paliers_du_bras(&points, Bras::Servi);
    let v = verdict(&table);

    // ── le rapport ──
    let md = rapport(&points, &table, &servi, &v, &endpoint, echecs, temperature);
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let _ = std::fs::write(base.join("MZ_MESURE_CADRAGE.md"), &md);
    let _ = std::fs::write(
        base.join("MZ_MESURE_CADRAGE.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "endpoint": endpoint,
            "points": points,
            "temperature": temperature,
            "echecs": echecs,
            "paliers_exige": table,
            "paliers_servi": servi,
            "verdict": v,
        }))
        .unwrap_or_default(),
    );

    eprintln!("\n{}\n", v.rendu());
    eprintln!("rapport : crates/scella-mz/MZ_MESURE_CADRAGE.md");

    // Sortie 0 : la mesure a eu lieu. Le verdict n'est PAS un code de retour — cf. la note du
    // module.
}

/// Re-fabrique le rapport depuis les mesures enregistrées, **sans appeler le modèle**.
///
/// Le point délicat : ne **rien** recopier de ce que le JSON contient déjà de dérivé. Paliers et
/// verdict sont recalculés depuis les `points` bruts, puis confrontés à ce qui était enregistré.
/// Recopier aurait fait un re-rendu qui ne peut pas échouer — et un garde qui ne peut pas tomber ne
/// garde rien.
///
/// N'écrit **que** le `.md`. Le `.json` est la mesure : le re-rendu le lit et n'y touche jamais.
///
/// Rend le code de sortie : `0` si le rendu a eu lieu ET reproduit le verdict enregistré, `1` si le
/// JSON manque ou est illisible, `3` si le recalcul **diverge** de l'enregistrement.
fn rendre_depuis_le_json() -> i32 {
    use scella_mz::mesure::{paliers_du_bras, verdict, Palier, Verdict};

    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let chemin = base.join("MZ_MESURE_CADRAGE.json");
    let brut = match std::fs::read_to_string(&chemin) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("MZ --rendre : {} illisible : {e}", chemin.display());
            eprintln!("Il n'y a rien à re-rendre tant qu'aucun balayage n'a eu lieu.");
            return 1;
        }
    };
    let j: serde_json::Value = match serde_json::from_str(&brut) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("MZ --rendre : JSON malformé : {e}");
            return 1;
        }
    };

    let points: Vec<Point> = match serde_json::from_value(j["points"].clone()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("MZ --rendre : `points` illisible : {e}");
            return 1;
        }
    };
    if points.is_empty() {
        eprintln!("MZ --rendre : aucun point enregistré.");
        return 1;
    }
    let endpoint = j["endpoint"].as_str().unwrap_or("(non enregistré)").to_string();
    let temperature = j["temperature"].as_f64().unwrap_or(0.0) as f32;
    // Absent des JSON produits avant que ce mode existe : on le dit, on ne l'invente pas.
    let echecs_connus = j["echecs"].as_u64().map(|n| n as usize);
    let echecs = echecs_connus.unwrap_or(0);

    // RECALCUL, pas recopie.
    let table = paliers_du_bras(&points, Bras::Exige);
    let servi = paliers_du_bras(&points, Bras::Servi);
    let v = verdict(&table);

    // Confrontation à l'enregistrement — c'est ce qui rend le re-rendu falsifiable.
    let mut divergences: Vec<String> = Vec::new();
    if let Ok(ancien) = serde_json::from_value::<Verdict>(j["verdict"].clone()) {
        if !ancien.meme_issue(&v) {
            divergences.push(format!("verdict : enregistré {ancien:?}, recalculé {v:?}"));
        }
    }
    for (nom, calc) in [("paliers_exige", &table), ("paliers_servi", &servi)] {
        if let Ok(anciens) = serde_json::from_value::<Vec<Palier>>(j[nom].clone()) {
            if anciens.len() != calc.len() {
                divergences.push(format!(
                    "{nom} : {} paliers enregistrés, {} recalculés",
                    anciens.len(),
                    calc.len()
                ));
                continue;
            }
            for (a, c) in anciens.iter().zip(calc.iter()) {
                if a.cible_mots != c.cible_mots
                    || (a.ratio_median - c.ratio_median).abs() > 1e-9
                    || (a.preservation_verifiee - c.preservation_verifiee).abs() > 1e-9
                    || (a.preservation_declaree - c.preservation_declaree).abs() > 1e-9
                    || (a.degenerescence - c.degenerescence).abs() > 1e-9
                    || a.declarations_infondees != c.declarations_infondees
                {
                    divergences.push(format!(
                        "{nom} · cible {} : enregistré ≠ recalculé",
                        a.cible_mots
                    ));
                }
            }
        }
    }

    let md = rapport(&points, &table, &servi, &v, &endpoint, echecs, temperature);
    if let Err(e) = std::fs::write(base.join("MZ_MESURE_CADRAGE.md"), &md) {
        eprintln!("MZ --rendre : écriture impossible : {e}");
        return 1;
    }

    // ⚠ Le JSON n'est PAS ré-écrit, et ce n'est pas une économie : c'est une correction.
    //
    // Une première version rafraîchissait ici les champs dérivés « puisque ce sont des caches
    // d'une fonction pure ». Elle relisait donc `points` pour les ré-émettre — et `scripts/
    // mz-check.sh` a montré qu'ils en ressortaient CHANGÉS : `5/11` enregistré
    // `0.45454545454545453` revenait `0.4545454545454546`. La lecture de flottants de `serde_json`
    // n'est pas correctement arrondie par défaut (son chemin rapide peut manquer d'1 ulp ; c'est
    // ce que corrige sa feature `float_roundtrip`, qu'on ne peut pas activer ici sans l'imposer
    // aux 67 crates de l'espace de travail — l'unification des features de Cargo est globale).
    //
    // La valeur abîmée est ensuite un point fixe : le second passage confirme le premier. Un
    // aller-retour « lire, modifier, réécrire » réécrit ainsi, en silence, des nombres qu'il n'a
    // jamais touchés. Le remède ne demande aucun drapeau : **on n'écrit pas un fichier dont on
    // n'est pas l'auteur.** Le balayage écrit le `.json` ; `--rendre` écrit le `.md`.
    //
    // Ce que cela laisse : les raisons enregistrées gardent la formulation du balayage qui les a
    // produites. C'est exact — elles rapportent ce que CE balayage a conclu — et `meme_issue` dit
    // déjà qu'une formulation n'est pas une mesure.

    eprintln!("MZ --rendre : {} mesures relues, aucun appel au modèle.", points.len());
    if echecs_connus.is_none() {
        eprintln!("  ⚠ `echecs` absent du JSON (balayage antérieur à ce mode) — rendu à 0.");
    }
    if divergences.is_empty() {
        eprintln!("  paliers et verdict RECALCULÉS ≡ enregistrés — le rapport dérive bien des mesures.");
        eprintln!("\n{}\n", v.rendu());
        eprintln!("rapport : crates/scella-mz/MZ_MESURE_CADRAGE.md");
        0
    } else {
        eprintln!("  ⚠ DIVERGENCE — le rapport ne dérive plus des mesures :");
        for d in &divergences {
            eprintln!("    · {d}");
        }
        3
    }
}

/// Le premier document de sa famille dans le corpus — pour n'éprouver le déterminisme qu'une fois
/// par famille (chaque essai coûte un appel de plus).
fn premier_de_sa_famille(id: &str, f: Famille) -> bool {
    CORPUS.iter().find(|d| d.famille == f).map(|d| d.id) == Some(id)
}

fn rapport(
    points: &[Point],
    exige: &[scella_mz::mesure::Palier],
    servi: &[scella_mz::mesure::Palier],
    v: &scella_mz::mesure::Verdict,
    endpoint: &str,
    echecs: usize,
    temperature: f32,
) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = writeln!(s, "# MZ — la mesure du Cadrage\n");
    let _ = writeln!(s, "> compresseur : `{endpoint}`  ");
    let _ = writeln!(s, "> température : `{temperature}`  ");
    let _ = writeln!(s, "> délai par appel : `{} s` (défaut client : 5 s)  ", DELAI.as_secs());
    let _ = writeln!(s, "> {} mesures, {echecs} échec(s)\n", points.len());

    let _ = writeln!(s, "## Verdict\n\n```\n{}\n```\n", v.rendu());
    let _ = writeln!(
        s,
        "Le verdict porte sur le bras **exigé** — là où la promesse du contrat est éprouvée. Juger \
         le compresseur sur une configuration où on ne lui demande rien l'acquitterait par \
         construction.\n"
    );

    let table = |titre: &str, note: &str, t: &[scella_mz::mesure::Palier], s: &mut String| {
        let _ = writeln!(s, "### {titre}\n\n{note}\n");
        let _ = writeln!(
            s,
            "| cible | ratio médian | préservation **déclarée** | préservation **vérifiée** | écart | dégénérescence | déclarations infondées |"
        );
        let _ = writeln!(s, "|---|---|---|---|---|---|---|");
        for p in t {
            let _ = writeln!(
                s,
                "| {} mots | {:.1}:1 | {:.1} % | **{:.1} %** | {:+.1} pp | {:.1} % | {} |",
                p.cible_mots,
                p.ratio_median,
                p.preservation_declaree * 100.0,
                p.preservation_verifiee * 100.0,
                p.ecart * 100.0,
                p.degenerescence * 100.0,
                p.declarations_infondees
            );
        }
        let _ = writeln!(s);
    };

    let _ = writeln!(s, "## Les deux bras\n");
    table(
        "Bras `exigé` — la promesse",
        "`must_preserve` = les entités du corpus. Le filtre du client (`validate_preservation`) est \
         **actif** : il rejette l'appel quand la liste déclarée ne les couvre pas. Conséquence à \
         garder en tête — ce bras ne peut enregistrer que des réponses où le modèle a **déclaré** \
         tout préserver ; une omission avouée y devient une erreur, pas une mesure.",
        exige,
        &mut s,
    );
    table(
        "Bras `servi` — ce que le pipeline obtient",
        "`must_preserve` = `vec![]`, **la configuration réellement exécutée** : les deux sites de \
         production passent une liste vide. Le filtre du client est alors une boucle sur rien, donc \
         il n'écarte plus personne — et la vérification indépendante de MZ est le **seul** contrôle \
         qui subsiste. C'est le chiffre à opposer à qui décrit le Cadrage tel qu'il tourne.",
        servi,
        &mut s,
    );

    // ── ⭐ La comparaison des bras : l'effet RÉEL de `must_preserve` ──
    let _ = writeln!(s, "\n## L'effet de `must_preserve`, mesuré\n");
    let _ = writeln!(
        s,
        "Le contrat annonce « préservant des entités **spécifiées** ». Voici ce que le fait de les \
         spécifier change réellement, à cible égale :\n"
    );
    let _ = writeln!(s, "| cible | vérifié si l'on DEMANDE | vérifié si l'on ne demande PAS | effet |");
    let _ = writeln!(s, "|---|---|---|---|");
    let mut ecarts = Vec::new();
    for e in exige {
        if let Some(sv) = servi.iter().find(|x| x.cible_mots == e.cible_mots) {
            let d = (e.preservation_verifiee - sv.preservation_verifiee) * 100.0;
            ecarts.push(d);
            let _ = writeln!(
                s,
                "| {} mots | {:.1} % | {:.1} % | **{:+.1} pp** |",
                e.cible_mots,
                e.preservation_verifiee * 100.0,
                sv.preservation_verifiee * 100.0,
                d
            );
        }
    }
    if !ecarts.is_empty() {
        let moy: f64 = ecarts.iter().sum::<f64>() / ecarts.len() as f64;
        let _ = writeln!(
            s,
            "\nEffet moyen de la demande : **{moy:+.1} pp**.\n\n\
             Si cet effet est nul, la conclusion est plus dure que « le compresseur perd des \
             entités » : le paramètre `must_preserve` est **inerte**. Le champ, l'instruction du \
             prompt qui dit « keeping every entity from must_preserve », et la validation qui le \
             relit — aucun des trois ne change ce qui sort. Et le modèle n'est pas malhonnête, il \
             est **obéissant** : quand on lui demande X il déclare « j'ai gardé X », quand on ne \
             demande rien il déclare 0 %. Le champ `preserved` **recopie la demande** ; il ne \
             rapporte pas la réalité. C'est pourquoi `validate_preservation`, qui ne lit que lui, \
             est structurellement incapable de détecter une perte.\n"
        );
    }

    let _ = writeln!(s, "\n## Ce qui a été perdu, nommé\n");
    let mut perdues: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for p in points {
        for e in &p.perdues {
            *perdues.entry(e.clone()).or_insert(0) += 1;
        }
    }
    if perdues.is_empty() {
        let _ = writeln!(s, "Aucune entité perdue sur l'ensemble du balayage.\n");
    } else {
        let _ = writeln!(s, "| entité | perdue N fois |\n|---|---|");
        let mut v: Vec<_> = perdues.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for (e, n) in v.iter().take(20) {
            let _ = writeln!(s, "| `{e}` | {n} |");
        }
        let _ = writeln!(s);
    }

    let stables: Vec<&Point> = points.iter().filter(|p| p.stable.is_some()).collect();
    if !stables.is_empty() {
        let n_ok = stables.iter().filter(|p| p.stable == Some(true)).count();
        let _ = writeln!(
            s,
            "## Déterminisme\n\n{n_ok}/{} paires identiques. La campagne H1.1 avait mesuré une \
             compression **non déterministe** à `temperature 0.2` (≈ 33 % de no-op) ; le passage au \
             greedy (0.0) était censé la supprimer. Rejouer avec \
             `EFE_COMPRESSOR_TEMPERATURE=0.2` donne le bras de comparaison.\n",
            stables.len()
        );
    }

    let _ = writeln!(
        s,
        "## Ce que cette mesure ne dit pas\n\n\
         - La **qualité de la réponse finale**. MZ mesure le compresseur, pas le pipeline.\n\
         - La **correction**. Ce qui est mesuré est ce qui a été *préservé*, jamais ce qui est *vrai*.\n\
         - Les **autres robots**. Analyseur, vérifieur et reformulateur ont leurs propres plafonds.\n\n\
         Et une limite de la mesure elle-même, qu'il faut dire : sur le bras **exigé**, le filtre du \
         client écarte les réponses où le modèle avoue une omission. Ce bras est donc, par \
         construction, biaisé vers les réponses qui déclarent tout préserver. C'est précisément \
         pourquoi le bras **servi** existe — il n'a aucun filtre.\n"
    );
    s
}
