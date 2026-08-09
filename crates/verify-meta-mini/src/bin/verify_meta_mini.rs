// SPDX-License-Identifier: UNLICENSED
//! # `verify_meta_mini` — re-preuve **zéro-dépendance** d'un consensus de méta-essaim.
//!
//! Un tiers re-compile ce binaire (`cargo tree --edges normal` = lui seul), coupe le réseau, et re-prouve
//! un bundle de consensus méta en ne faisant confiance **qu'aux maths** et à sa propre compilation.
//!
//! Usage : `verify_meta_mini <meta_steps.jsonl> <meta_manifest.json> --key-hex <64hex> [--json]`
//! Codes de sortie : `0` intègre · `1` altéré/réordonné/mauvaise clé (localisé) · `2` usage/E-S.

use std::process::exit;
use verify_meta_mini::json::parse_object;
use verify_meta_mini::verify::{from_hex, from_hex32, to_hex, verify, Outcome, Step};

const USAGE: &str = "usage : verify_meta_mini <meta_steps.jsonl> <meta_manifest.json> \
[--key-hex <64hex>] [--anchor <journal>] [--json]\n\
\n\
  --key-hex   RE-PROUVE la chaîne (exige la clé du tenant).\n\
  --anchor    RECOUPE la racine et le nombre de pas avec le journal publié.\n\
              Ne demande AUCUN SECRET : c\'est ce qui permet à un tiers sans clé\n\
              de constater une TRONCATURE, que la re-preuve seule ne voit pas.\n\
  Au moins l\'un des deux est requis.";

fn main() {
    exit(real_main(&std::env::args().collect::<Vec<_>>()));
}

fn real_main(args: &[String]) -> i32 {
    let (mut steps_path, mut manifest_path, mut key_hex, mut json) = (None, None, None, false);
    let mut anchor_path: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--key-hex" => {
                i += 1;
                key_hex = args.get(i).cloned();
            }
            "--anchor" => {
                i += 1;
                anchor_path = args.get(i).cloned();
            }
            "--json" => json = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            other if steps_path.is_none() => steps_path = Some(other.to_string()),
            other if manifest_path.is_none() => manifest_path = Some(other.to_string()),
            other => {
                eprintln!("argument inattendu : {other}\n\n{USAGE}");
                return 2;
            }
        }
        i += 1;
    }
    let (steps_path, manifest_path) = match (steps_path, manifest_path) {
        (Some(a), Some(b)) => (a, b),
        _ => {
            eprintln!("erreur : arguments manquants\n\n{USAGE}");
            return 2;
        }
    };
    // Sans clé ET sans ancre, il n'y a rien à constater : refuser vaut mieux que rendre un « OK »
    // qui ne reposerait sur aucun contrôle.
    if key_hex.is_none() && anchor_path.is_none() {
        eprintln!("erreur : ni --key-hex ni --anchor — aucun contrôle à effectuer\n\n{USAGE}");
        return 2;
    }
    // Clé absente ⟹ on ne re-prouvera pas la chaîne, on se limitera au recoupement d'ancre. Le
    // verdict le dira EXPLICITEMENT plutôt que de laisser croire à une vérification complète.
    let key_hex = key_hex.unwrap_or_default();
    let reprove = !key_hex.is_empty();
    let key = if reprove {
        match from_hex(&key_hex) {
            Some(k) if !k.is_empty() => k,
            _ => {
                eprintln!("erreur : --key-hex invalide (chaîne hex non vide attendue)");
                return 2;
            }
        }
    } else {
        Vec::new()
    };

    // Les pas (JSONL, une SealedMetaStep par ligne).
    let steps_raw = match std::fs::read_to_string(&steps_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("erreur : impossible de lire « {steps_path} » : {e}");
            return 2;
        }
    };
    let mut steps = Vec::new();
    for (n, line) in steps_raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match Step::from_json_line(line) {
            Ok(s) => steps.push(s),
            Err(e) => {
                eprintln!("erreur : ligne {n} : {e}");
                return 2;
            }
        }
    }

    // Le manifeste (objet JSON, éventuellement multi-ligne).
    let manifest_raw = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("erreur : impossible de lire « {manifest_path} » : {e}");
            return 2;
        }
    };
    let m = match parse_object(manifest_raw.trim()) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("erreur : manifeste illisible : {e}");
            return 2;
        }
    };
    let (meta_id, n_steps, expected_root) = match (m.get_str("meta_id"), m.get_u64("n_steps"), m.get_str("root")) {
        (Ok(id), Ok(n), Ok(r)) => match from_hex32(r) {
            Some(root) => (id.to_string(), n as usize, root),
            None => {
                eprintln!("erreur : root du manifeste : 64 hex attendus");
                return 2;
            }
        },
        _ => {
            eprintln!("erreur : manifeste : champs meta_id/n_steps/root attendus");
            return 2;
        }
    };

    if n_steps != steps.len() {
        eprintln!("erreur : manifeste incohérent (n_steps={n_steps} ≠ {} lignes)", steps.len());
        return 2;
    }

    // **v4** — recouper la LIAISON (qui / quelle config / quelle règle) AVANT la chaîne.
    //
    // Sans ce contrôle, un manifeste dont on aurait réécrit les identités passerait la re-preuve
    // — qui ne regarde que le condensé scellé — et l'auditeur lirait des noms falsifiés sous un
    // root parfaitement valide. C'est le contrôle qui donne sa valeur au champ.
    match verify_meta_mini::verify::Binding::from_manifest(&m) {
        Ok(binding) => {
            if let Err(e) = verify_meta_mini::verify::check_binding(&steps, binding.as_ref()) {
                eprintln!("erreur : {e}");
                return 1;
            }
        }
        Err(e) => {
            eprintln!("erreur : {e}");
            return 2;
        }
    }

    // ── Couche 3 · 3a — le recoupement d'ANCRE, qui ne demande AUCUN SECRET ─────────────────
    //
    // Il vient AVANT la re-preuve parce qu'il attrape ce qu'elle ne voit pas : une TRONCATURE
    // laisse une chaîne parfaitement intègre, et seule une revendication publiée ailleurs la
    // révèle. Un tiers sans clé s'arrête ici, et cela lui apprend déjà quelque chose.
    if let Some(path) = &anchor_path {
        let journal = match std::fs::read_to_string(path) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("erreur : journal d'ancres « {path} » illisible : {e}");
                return 2;
            }
        };
        match verify_meta_mini::verify::find_anchor(&journal, &meta_id) {
            Err(e) => {
                eprintln!("erreur : {e}");
                return 2;
            }
            Ok(None) => {
                // Absence d'ancre : ce n'est PAS un échec d'intégrité, c'est une garantie qu'on
                // n'a pas. Le dire, plutôt que de rendre un vert qui la laisserait supposer.
                eprintln!(
                    "⚠ AUCUNE ANCRE publiée pour « {meta_id} » dans {path} — la troncature reste \
                     indétectable sans la clé."
                );
            }
            Ok(Some(anchor)) => {
                let root_hex = to_hex(&expected_root);
                if let Err(why) =
                    verify_meta_mini::verify::check_against_anchor(&root_hex, n_steps, &anchor)
                {
                    eprintln!("✗ ANCRE DISCORDANTE — {why}");
                    return 1;
                }
                println!(
                    "✓ ANCRE CONCORDANTE — racine et nombre de pas ({n_steps}) conformes à la \
                     revendication publiée, **sans clé**."
                );
            }
        }
        // Sans clé, le recoupement d'ancre est TOUT ce qu'on peut dire. Le dire, et s'arrêter là :
        // laisser croire à une vérification complète serait le contraire du but.
        if !reprove {
            println!(
                "  (chaîne NON re-prouvée : aucune --key-hex fournie. L'intégrité des pas n'est \
                 donc PAS attestée ici — seule la concordance avec l'ancre l'est.)"
            );
            return 0;
        }
    }

    let outcome = verify(&steps, &key, &expected_root);
    let ok = matches!(outcome, Outcome::Ok { .. });
    if json {
        println!("{}", json_out(&outcome, &meta_id, n_steps));
    } else {
        human(&outcome, &meta_id, n_steps);
    }
    i32::from(!ok)
}

fn human(o: &Outcome, meta_id: &str, n: usize) {
    match o {
        Outcome::Ok { root } => {
            println!("✓ VÉRIFIÉ — consensus méta-essaim « {meta_id} » authentique et non-altéré ({n} pas, root concordant).");
            println!("  meta_provenance_root : {}", to_hex(root));
            println!("  (zéro dépendance : recompilez ce vérifieur et re-prouvez sans nous)");
        }
        Outcome::Broken { at, reason } => {
            println!("✗ ÉCHEC — re-preuve refusée (contenu altéré, réordonné, mauvaise clé, ou manifeste incohérent).");
            println!("  cassé au pas : {at}");
            println!("  motif : {reason}");
        }
    }
}

fn json_out(o: &Outcome, meta_id: &str, n: usize) -> String {
    match o {
        Outcome::Ok { root } => format!("{{\"ok\":true,\"meta_id\":{meta_id:?},\"n_steps\":{n},\"root\":{:?},\"broken_at\":null}}", to_hex(root)),
        Outcome::Broken { at, reason } => format!("{{\"ok\":false,\"meta_id\":{meta_id:?},\"n_steps\":{n},\"broken_at\":{at},\"reason\":{reason:?}}}"),
    }
}
