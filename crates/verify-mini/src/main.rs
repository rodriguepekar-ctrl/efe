// SPDX-License-Identifier: UNLICENSED
//! # `verify-mini` (binaire) — le vérifieur forensic à **zéro dépendance**.
//!
//! Même contrat CLI que `provenance-verify`, mais l'arbre de dépendances est **vide** : tout (crypto,
//! parsing JSONL, encodage canonique, re-preuve, sortie JSON) est dans cette crate, sans serde. Un tiers
//! sceptique le **re-compile** et vérifie **sans rien croire**. Codes de sortie : `0` intègre · `1` échec
//! · `2` usage/E-S.

use std::process::exit;
use verify_mini::{run, Verdict};

fn main() {
    exit(real_main(&std::env::args().collect::<Vec<_>>()));
}

/// Le point d'entrée testable : prend les args, rend un code de sortie. N'imprime que des verdicts/erreurs.
fn real_main(args: &[String]) -> i32 {
    let opts = match Opts::parse(&args[1..]) {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("erreur : {msg}\n\n{USAGE}");
            return 2;
        }
    };

    let jsonl = match std::fs::read_to_string(&opts.records_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("erreur : impossible de lire « {} » : {e}", opts.records_path);
            return 2;
        }
    };
    let key = match opts.key_bytes() {
        Ok(k) => k,
        Err(msg) => {
            eprintln!("erreur : {msg}");
            return 2;
        }
    };

    match run(&jsonl, &key, opts.expected_root.as_deref()) {
        Ok(verdict) => {
            if opts.json {
                println!("{}", verdict_json(&verdict));
            } else {
                println!("{}", verdict_human(&verdict, opts.expected_root.is_some()));
            }
            if verdict.ok {
                0
            } else {
                1
            }
        }
        Err(msg) => {
            eprintln!("erreur : {msg}");
            2
        }
    }
}

// ── options CLI (parsing minimal, sans dépendance) ──

const USAGE: &str = "usage : verify-mini <records.jsonl> (--key-hex <64hex> | --key-file <chemin>) [--root <attendu>] [--json]";

struct Opts {
    records_path: String,
    key_hex: Option<String>,
    key_file: Option<String>,
    expected_root: Option<String>,
    json: bool,
}
impl Opts {
    fn parse(args: &[String]) -> Result<Opts, String> {
        let (mut records_path, mut key_hex, mut key_file, mut expected_root, mut json) = (None, None, None, None, false);
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            let mut take_val = |name: &str| -> Result<String, String> {
                i += 1;
                match args.get(i) {
                    // Refuse de « voler » l'option suivante comme valeur : `--root --json` est une erreur
                    // d'usage (exit 2), pas un faux verdict d'altération (exit 1).
                    Some(v) if v.starts_with("--") => Err(format!("{name} attend une valeur, mais «{v}» ressemble à une option")),
                    Some(v) => Ok(v.clone()),
                    None => Err(format!("{name} attend une valeur")),
                }
            };
            match a.as_str() {
                "--key-hex" => key_hex = Some(take_val("--key-hex")?),
                "--key-file" => key_file = Some(take_val("--key-file")?),
                "--root" => expected_root = Some(take_val("--root")?),
                "--json" => json = true,
                "-h" | "--help" => return Err("aide".into()),
                s if s.starts_with("--") => return Err(format!("option inconnue : {s}")),
                _ if records_path.is_none() => records_path = Some(a.clone()),
                _ => return Err(format!("argument positionnel en trop : {a}")),
            }
            i += 1;
        }
        let records_path = records_path.ok_or("chemin du JSONL de records manquant")?;
        if key_hex.is_some() == key_file.is_some() {
            return Err("fournir EXACTEMENT l'un de --key-hex ou --key-file".into());
        }
        Ok(Opts { records_path, key_hex, key_file, expected_root, json })
    }

    /// Les 32 octets de la clé (depuis --key-hex, ou le contenu hex d'un --key-file).
    fn key_bytes(&self) -> Result<[u8; 32], String> {
        let hex = match (&self.key_hex, &self.key_file) {
            (Some(h), None) => h.clone(),
            (None, Some(p)) => std::fs::read_to_string(p).map_err(|e| format!("lecture de la clé « {p} » : {e}"))?,
            _ => unreachable!("garanti par parse()"),
        };
        hex_to_32(hex.trim()).ok_or_else(|| "clé invalide : 64 caractères hexadécimaux attendus".into())
    }
}

/// 64 hex → 32 octets. Strict : exactement 64 chiffres hexa (rejette « +a… »).
fn hex_to_32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

// ── rendu du verdict ──

fn verdict_human(v: &Verdict, against_root: bool) -> String {
    if v.ok {
        format!(
            "✓ VÉRIFIÉ — record authentique et non-altéré ({} records{}).\n  manifest-root : {}\n  (intégrité attestée — PAS la correction de la décision, ni une reproductibilité bit-à-bit)",
            v.n_records,
            if against_root { ", root concordant" } else { "" },
            v.manifest_root.clone().unwrap_or_default()
        )
    } else {
        let loc = match v.broken_at {
            Some(i) => format!("\n  cassé au record : {i}"),
            None => String::new(),
        };
        format!(
            "✗ ÉCHEC — vérification refusée (record altéré/réordonné, clé/root incorrect, ou bundle vide).{}\n  motif : {}",
            loc,
            v.reason.clone().unwrap_or_default()
        )
    }
}

/// JSON du verdict, **fait main** (cohérent avec le reste du binaire : aucune dépendance).
fn verdict_json(v: &Verdict) -> String {
    let root = match &v.manifest_root {
        Some(r) => format!("\"{}\"", json_escape(r)),
        None => "null".into(),
    };
    let broken = v.broken_at.map(|i| i.to_string()).unwrap_or_else(|| "null".into());
    let reason = match &v.reason {
        Some(r) => format!("\"{}\"", json_escape(r)),
        None => "null".into(),
    };
    format!("{{\"ok\":{},\"n_records\":{},\"manifest_root\":{},\"broken_at\":{},\"reason\":{}}}", v.ok, v.n_records, root, broken, reason)
}

/// Échappement JSON minimal (guillemets, backslash, contrôles).
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_hex() -> String {
        [7u8; 32].iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn hex_parsing_is_strict() {
        assert_eq!(hex_to_32(&"00".repeat(32)), Some([0u8; 32]));
        assert_eq!(hex_to_32("abc"), None, "longueur");
        assert_eq!(hex_to_32(&"zz".repeat(32)), None, "non-hex");
        assert_eq!(hex_to_32(&"+a".repeat(32)), None, "« +a » non-canonique rejeté");
    }

    #[test]
    fn opts_parsing_mirrors_provenance_verify() {
        let o = Opts::parse(&["recs.jsonl".into(), "--key-hex".into(), "00".repeat(32), "--json".into()]).unwrap();
        assert_eq!(o.records_path, "recs.jsonl");
        assert!(o.json && o.key_hex.is_some());
        assert!(Opts::parse(&["r.jsonl".into()]).is_err(), "clé manquante");
        assert!(Opts::parse(&["r.jsonl".into(), "--key-hex".into(), "x".into(), "--key-file".into(), "p".into()]).is_err(), "deux clés");
        assert!(Opts::parse(&["r.jsonl".into(), "--root".into(), "--json".into()]).is_err(), "flag-comme-valeur refusé");
    }

    #[test]
    fn exit_codes_2_on_usage_and_io() {
        assert_eq!(real_main(&["prog".into()]), 2, "args manquants → 2");
        assert_eq!(real_main(&["prog".into(), "/nexistepas/x.jsonl".into(), "--key-hex".into(), key_hex()]), 2, "fichier illisible → 2");
    }

    #[test]
    fn json_verdict_is_balanced() {
        let v = Verdict { ok: false, n_records: 0, manifest_root: None, broken_at: Some(2), reason: Some("motif \"piégé\"".into()) };
        let j = verdict_json(&v);
        assert!(j.contains("\"ok\":false") && j.contains("\"broken_at\":2"));
        assert_eq!(j.matches('"').count() % 2, 0, "guillemets équilibrés : {j}");
    }
}
