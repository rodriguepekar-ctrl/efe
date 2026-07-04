// SPDX-License-Identifier: UNLICENSED
//! # `verify_meta` — re-preuve AUTONOME d'un consensus de méta-essaim (2 fichiers + 1 clé).
//!
//! Un tiers sceptique re-compile ce binaire et re-prouve, **hors-ligne**, qu'une suite de
//! pas de consensus de méta-essaim est authentique et non-altérée — **sans nous**. Il ne
//! fait confiance qu'aux maths (le sceau HMAC/BLAKE3, byte-identique à nano/essaim) et à
//! sa propre compilation. La **clé tenant** vient du responsable de traitement, jamais des
//! fichiers.
//!
//! Usage : `verify_meta <meta_steps.jsonl> <meta_manifest.json> --key-hex <64hex> [--json]`
//! Codes de sortie : `0` intègre · `1` altéré/réordonné/mauvaise clé (localisé) · `2` usage/E-S.

use efe_meta_verify::{from_hex, verify, MetaManifest, SealedMetaStep, Verdict};
use std::process::exit;

const USAGE: &str = "usage : verify_meta <meta_steps.jsonl> <meta_manifest.json> --key-hex <64hex> [--json]";

fn main() {
    exit(real_main(&std::env::args().collect::<Vec<_>>()));
}

/// Point d'entrée testable : args → code de sortie. N'imprime que des verdicts/erreurs.
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

    let steps = match read_steps(&steps_path) {
        Ok(s) => s,
        Err(code) => return code,
    };

    let manifest_raw = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("erreur : impossible de lire « {manifest_path} » : {e}");
            return 2;
        }
    };
    let manifest: MetaManifest = match serde_json::from_str(&manifest_raw) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("erreur : manifeste illisible : {e}");
            return 2;
        }
    };

    // Garde de forme : le manifeste doit décrire exactement ces pas.
    if manifest.n_steps != steps.len() {
        eprintln!("erreur : manifeste incohérent (n_steps={} ≠ {} lignes)", manifest.n_steps, steps.len());
        return 2;
    }

    let verdict = verify(&steps, &key, &manifest.root);
    if json {
        println!("{}", verdict_json(&verdict, &manifest));
    } else {
        print_human(&verdict, &manifest);
    }
    i32::from(!verdict.ok) // 0 intègre, 1 échec
}

/// Lit et parse le `meta_steps.jsonl` (une [`SealedMetaStep`] par ligne non vide).
fn read_steps(path: &str) -> Result<Vec<SealedMetaStep>, i32> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        eprintln!("erreur : impossible de lire « {path} » : {e}");
        2
    })?;
    let mut out = Vec::new();
    for (n, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(s) => out.push(s),
            Err(e) => {
                eprintln!("erreur : ligne JSONL {n} illisible : {e}");
                return Err(2);
            }
        }
    }
    Ok(out)
}

fn print_human(v: &Verdict, m: &MetaManifest) {
    if v.ok {
        println!("✓ VÉRIFIÉ — consensus méta-essaim « {} » authentique et non-altéré ({} pas, root concordant).", m.meta_id, m.n_steps);
        println!("  meta_provenance_root : {}", v.root);
        println!("  (intégrité attestée — PAS la correction de la décision de consensus)");
    } else {
        println!("✗ ÉCHEC — re-preuve refusée (contenu altéré, réordonné, mauvaise clé, ou manifeste incohérent).");
        if let Some(k) = v.broken_at {
            println!("  cassé au pas : {k}");
        }
        if let Some(r) = &v.reason {
            println!("  motif : {r}");
        }
    }
}

fn verdict_json(v: &Verdict, m: &MetaManifest) -> String {
    let broken = v.broken_at.map_or_else(|| "null".to_string(), |x| x.to_string());
    let reason = v.reason.as_ref().map_or_else(|| "null".to_string(), |r| format!("{r:?}"));
    format!(
        "{{\"ok\":{},\"meta_id\":{:?},\"n_steps\":{},\"root\":{:?},\"broken_at\":{broken},\"reason\":{reason}}}",
        v.ok, m.meta_id, m.n_steps, v.root
    )
}
