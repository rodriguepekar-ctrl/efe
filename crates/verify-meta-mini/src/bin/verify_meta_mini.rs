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

const USAGE: &str = "usage : verify_meta_mini <meta_steps.jsonl> <meta_manifest.json> --key-hex <64hex> [--json]";

fn main() {
    exit(real_main(&std::env::args().collect::<Vec<_>>()));
}

fn real_main(args: &[String]) -> i32 {
    let (mut steps_path, mut manifest_path, mut key_hex, mut json) = (None, None, None, false);
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--key-hex" => {
                i += 1;
                key_hex = args.get(i).cloned();
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
    let (steps_path, manifest_path, key_hex) = match (steps_path, manifest_path, key_hex) {
        (Some(a), Some(b), Some(k)) => (a, b, k),
        _ => {
            eprintln!("erreur : arguments manquants\n\n{USAGE}");
            return 2;
        }
    };
    let key = match from_hex(&key_hex) {
        Some(k) if !k.is_empty() => k,
        _ => {
            eprintln!("erreur : --key-hex invalide (chaîne hex non vide attendue)");
            return 2;
        }
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
