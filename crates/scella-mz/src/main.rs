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

use std::sync::Arc;
use std::time::Instant;

use efe_nano_app_coordinator::types::{CompressionStyle, CompressorInput};
use efe_nano_app_coordinator::{
    clients::compressor::CompressorClient, CallContext, InMemoryAuditWriter, NanoRobotClient,
};
use scella_mz::corpus::{Famille, CORPUS};
use scella_mz::mesure::{paliers, verdict, Point};
use scella_mz::verifieur::{taux_declare, taux_verifie};

/// Les cibles balayées, du généreux à l'agressif. Un document de ~330 mots donne ainsi des ratios
/// d'environ 1,7:1 à 16:1 — la fourchette revendiquée (8:1 à 12:1) est **encadrée**, ce qui est la
/// seule façon de savoir de quel côté le plafond tombe.
const CIBLES: &[u32] = &[200, 120, 80, 50, 30, 20];

/// Le document sur lequel on éprouve le déterminisme, par famille : deux appels identiques.
const CIBLE_DETERMINISME: u32 = 50;

#[tokio::main]
async fn main() {
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

    let client = match CompressorClient::from_assets(&endpoint) {
        Ok(c) => c,
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

    for cible in CIBLES {
        for doc in CORPUS {
            let entree = CompressorInput {
                text: doc.texte.to_string(),
                target_words: *cible,
                // ⭐ On demande la préservation — ce que le pipeline servi ne fait JAMAIS
                //   (`must_preserve: vec![]` aux deux sites). Sans cela, on mesurerait la même
                //   chose que le rapport existant : rien sur la préservation.
                must_preserve: doc.entites.iter().map(|e| (*e).to_string()).collect(),
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
                            must_preserve: doc.entites.iter().map(|e| (*e).to_string()).collect(),
                            style: CompressionStyle::Extractive,
                        };
                        p.stable = client.invoke(bis, &ctx).await.ok().map(|b| b.summary == o.summary);
                    }

                    eprintln!(
                        "  {:<10} cible {:>3}  ratio {:>5.1}:1  déclaré {:>5.1} %  vérifié {:>5.1} % {} {}",
                        doc.id,
                        cible,
                        p.ratio,
                        p.preservation_declaree * 100.0,
                        p.preservation_verifiee * 100.0,
                        if p.ment() { "⚠ MENT" } else { "" },
                        if p.degenere { "⚠ DÉGÉNÉRÉ" } else { "" }
                    );
                    points.push(p);
                }
                Err(e) => {
                    echecs += 1;
                    eprintln!("  {:<10} cible {:>3}  ÉCHEC : {e}", doc.id, cible);
                }
            }
        }
    }

    if points.is_empty() {
        eprintln!("\nMESURE IMPOSSIBLE — aucun appel n'a abouti ({echecs} échec(s)).");
        eprintln!("  Le serveur tourne-t-il ?  llama-server -m models/compressor.gguf --port 8081");
        std::process::exit(2);
    }

    let table = paliers(&points);
    let v = verdict(&table);

    // ── le rapport ──
    let md = rapport(&points, &table, &v, &endpoint, echecs);
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let _ = std::fs::write(base.join("MZ_MESURE_CADRAGE.md"), &md);
    let _ = std::fs::write(
        base.join("MZ_MESURE_CADRAGE.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "endpoint": endpoint,
            "points": points,
            "paliers": table,
            "verdict": v,
        }))
        .unwrap_or_default(),
    );

    eprintln!("\n{}\n", v.rendu());
    eprintln!("rapport : crates/scella-mz/MZ_MESURE_CADRAGE.md");

    // Sortie 0 : la mesure a eu lieu. Le verdict n'est PAS un code de retour — cf. la note du
    // module.
}

/// Le premier document de sa famille dans le corpus — pour n'éprouver le déterminisme qu'une fois
/// par famille (chaque essai coûte un appel de plus).
fn premier_de_sa_famille(id: &str, f: Famille) -> bool {
    CORPUS.iter().find(|d| d.famille == f).map(|d| d.id) == Some(id)
}

fn rapport(
    points: &[Point],
    table: &[scella_mz::mesure::Palier],
    v: &scella_mz::mesure::Verdict,
    endpoint: &str,
    echecs: usize,
) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = writeln!(s, "# MZ — la mesure du Cadrage\n");
    let _ = writeln!(s, "> compresseur : `{endpoint}`  ");
    let _ = writeln!(
        s,
        "> température : `{}`  ",
        std::env::var("EFE_COMPRESSOR_TEMPERATURE").unwrap_or_else(|_| "0.0 (défaut, greedy)".into())
    );
    let _ = writeln!(s, "> {} mesures, {echecs} échec(s)\n", points.len());

    let _ = writeln!(s, "## Verdict\n\n```\n{}\n```\n", v.rendu());

    let _ = writeln!(s, "## La courbe d'arbitrage\n");
    let _ = writeln!(
        s,
        "| cible | ratio médian | préservation **déclarée** | préservation **vérifiée** | écart | dégénérescence | menteurs |"
    );
    let _ = writeln!(s, "|---|---|---|---|---|---|---|");
    for p in table {
        let _ = writeln!(
            s,
            "| {} mots | {:.1}:1 | {:.1} % | **{:.1} %** | {:+.1} pp | {:.1} % | {} |",
            p.cible_mots,
            p.ratio_median,
            p.preservation_declaree * 100.0,
            p.preservation_verifiee * 100.0,
            p.ecart * 100.0,
            p.degenerescence * 100.0,
            p.menteurs
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
        v.sort_by(|a, b| b.1.cmp(&a.1));
        for (e, n) in v.iter().take(20) {
            let _ = writeln!(s, "| `{e}` | {n} |");
        }
    }

    let stables: Vec<&Point> = points.iter().filter(|p| p.stable.is_some()).collect();
    if !stables.is_empty() {
        let n_ok = stables.iter().filter(|p| p.stable == Some(true)).count();
        let _ = writeln!(
            s,
            "\n## Déterminisme\n\n{n_ok}/{} paires identiques. La campagne H1.1 avait mesuré une \
             compression **non déterministe** à `temperature 0.2` (≈ 33 % de no-op) ; le passage au \
             greedy (0.0) était censé la supprimer.\n",
            stables.len()
        );
    }

    let _ = writeln!(
        s,
        "## Ce que cette mesure ne dit pas\n\n\
         - La **qualité de la réponse finale**. MZ mesure le compresseur, pas le pipeline.\n\
         - La **correction**. Ce qui est mesuré est ce qui a été *préservé*, jamais ce qui est *vrai*.\n\
         - Les **autres robots**. Analyseur, vérifieur et reformulateur ont leurs propres plafonds.\n\n\
         Et un fait qui encadre tout le reste : les deux sites de production du pipeline passent \
         `must_preserve: vec![]`. Cette mesure demande la préservation ; **le service, jamais**.\n"
    );
    s
}
