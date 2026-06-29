// SPDX-License-Identifier: UNLICENSED
//! # `provenance-verify` — le vérifieur forensic AUTONOME (Kovant P1 / S4, le composant-fossé)
//!
//! Prend un **JSONL de records scellés** + la **clé tenant** (que le client détient), re-dérive et
//! **vérifie l'intégrité** (la re-preuve de S0), et rend un verdict + un code de sortie. Conçu pour le
//! **tiers sceptique** : il dépend **uniquement** de [`provenance_dag`] (zéro dépendance crypto externe),
//! est minuscule, **re-compilable**, et lisible en une heure — un régulateur, un auditeur ou un tribunal
//! peut vérifier **sans faire confiance** ni à l'éditeur ni à une dépendance. La sortie JSON est faite
//! main (pas de `serde`) pour que ce binaire ne dépende de RIEN d'autre que du cœur auditable.
//!
//! Usage :
//! ```text
//!   provenance-verify <records.jsonl> (--key-hex <64hex> | --key-file <chemin>) [--root <attendu>] [--json]
//! ```
//! Codes de sortie : `0` = vérifié intègre · `1` = échec (altéré/réordonné/mauvaise clé) · `2` = usage/E-S.
//!
//! **Sémantique** : atteste l'**INTÉGRITÉ** (le compte authentique, non-altéré) — **pas** la correction
//! de la décision, **pas** une reproductibilité bit-à-bit.

use std::process::exit;

use provenance_dag::{reprove_dag, verify_against_root, SealedDag, TenantKey, Verdict};

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
        Ok(k) => TenantKey::from_bytes(k),
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

/// Le cœur : parse les records, re-prouve (ou vérifie contre un root). `Err` = JSONL invalide.
fn run(records_jsonl: &str, key: &TenantKey, expected_root: Option<&str>) -> Result<Verdict, String> {
    let records = SealedDag::records_from_jsonl(records_jsonl).map_err(|e| format!("JSONL invalide : {e}"))?;
    // Un bundle VIDE (ou tronqué jusqu'au néant) ne doit JAMAIS être attesté « authentique » : une
    // chaîne vide re-prouve trivialement (genesis, clé jamais consultée). On le REFUSE explicitement —
    // attester l'authenticité de records qui n'existent pas est précisément le faux-positif à éviter.
    if records.is_empty() {
        return Ok(Verdict {
            ok: false,
            n_records: 0,
            manifest_root: None,
            broken_at: None,
            reason: Some("bundle vide — aucun record à attester (refus)".into()),
        });
    }
    Ok(match expected_root {
        Some(root) => verify_against_root(&records, key, root),
        None => reprove_dag(&records, key),
    })
}

// ── options CLI (parsing minimal, sans dépendance) ──

const USAGE: &str = "usage : provenance-verify <records.jsonl> (--key-hex <64hex> | --key-file <chemin>) [--root <attendu>] [--json]";

struct Opts {
    records_path: String,
    key_hex: Option<String>,
    key_file: Option<String>,
    expected_root: Option<String>,
    json: bool,
}
impl Opts {
    fn parse(args: &[String]) -> Result<Opts, String> {
        let (mut records_path, mut key_hex, mut key_file, mut expected_root, mut json) =
            (None, None, None, None, false);
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            let mut take_val = |name: &str| -> Result<String, String> {
                i += 1;
                match args.get(i) {
                    // Refuse de « voler » l'option suivante comme valeur : `--root --json` est une
                    // erreur d'usage (exit 2), pas un faux verdict d'altération (exit 1).
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

/// 64 hex → 32 octets. Strict : exactement 64 chiffres hexa (rejette « +a… » que `from_str_radix`
/// accepterait sinon, aliasant des chaînes distinctes vers la même clé).
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
        // `broken_at` n'existe que pour un maillon précis (altération/réordonnancement) ; il est absent
        // pour un échec global (root publié non concordant, bundle vide).
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

/// JSON du verdict, **fait main** (pas de dépendance serde dans ce binaire-fossé).
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
    format!(
        "{{\"ok\":{},\"n_records\":{},\"manifest_root\":{},\"broken_at\":{},\"reason\":{}}}",
        v.ok, v.n_records, root, broken, reason
    )
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
    use provenance_dag::{Node, SealedDag, TenantKey};

    /// Octets de clé FIXES et connus : la source de vérité unique pour `key()` ET `key_hex_of()`
    /// (on ne re-dérive PAS la formule de S0 dans le test — sinon un changement de formule ferait
    /// passer une clé fausse silencieusement).
    const KEY_BYTES: [u8; 32] = [7u8; 32];

    fn key() -> TenantKey {
        TenantKey::from_bytes(KEY_BYTES)
    }

    /// La clé en hex (pour les tests CLI de `real_main`) — dérivée des MÊMES octets que `key()`.
    fn key_hex_of() -> String {
        KEY_BYTES.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn sample_jsonl() -> (String, String) {
        let k = key();
        let mut d = SealedDag::new();
        d.append(&k, Node::AgentAct { node_id: "n1".into(), agent_role: "r".into(), inputs_hash: "h".into(), model: "m".into(), version: "1".into(), output: "PASS".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
        d.append(&k, Node::Human { node_id: "review:1".into(), human_id: "officer:1".into(), action: provenance_dag::HumanAction::Approve, target_node: "n1".into(), justification: "ok".into(), timestamp: 2 }).unwrap();
        (d.to_jsonl(), d.manifest_root())
    }

    #[test]
    fn run_ok_on_authentic_dag() {
        let (jsonl, root) = sample_jsonl();
        let v = run(&jsonl, &key(), None).unwrap();
        assert!(v.ok);
        assert_eq!(v.manifest_root.as_deref(), Some(root.as_str()));
        // contre le bon root.
        assert!(run(&jsonl, &key(), Some(&root)).unwrap().ok);
        // contre un mauvais root → échec.
        assert!(!run(&jsonl, &key(), Some(&"ff".repeat(32))).unwrap().ok);
    }

    #[test]
    fn run_fails_on_tampered_jsonl() {
        let (jsonl, _) = sample_jsonl();
        // Altère « PASS » → « FAIL » dans le JSONL brut (ce que ferait un falsificateur).
        let tampered = jsonl.replace("PASS", "FAIL");
        let v = run(&tampered, &key(), None).unwrap();
        assert!(!v.ok, "altération détectée");
        assert_eq!(v.broken_at, Some(0));
    }

    #[test]
    fn run_fails_on_wrong_key() {
        let (jsonl, _) = sample_jsonl();
        let other = TenantKey::derive(b"root", "globex");
        assert!(!run(&jsonl, &other, None).unwrap().ok, "mauvaise clé → échec");
    }

    #[test]
    fn run_errors_on_bad_jsonl() {
        assert!(run("ceci n'est pas du json", &key(), None).is_err());
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(hex_to_32(&"00".repeat(32)), Some([0u8; 32]));
        assert_eq!(hex_to_32(&"ff".repeat(32)), Some([0xff; 32]));
        assert_eq!(hex_to_32("abc"), None, "longueur invalide");
        assert_eq!(hex_to_32(&"zz".repeat(32)), None, "non-hex");
        // Strict : un préfixe de signe « + » (accepté par from_str_radix) est rejeté.
        assert_eq!(hex_to_32(&"+a".repeat(32)), None, "hex non-canonique « +a » rejeté");
        assert_eq!(hex_to_32(&"-a".repeat(32)), None, "signe « - » rejeté");
    }

    #[test]
    fn opts_parsing() {
        let o = Opts::parse(&["recs.jsonl".into(), "--key-hex".into(), "00".repeat(32), "--json".into()]).unwrap();
        assert_eq!(o.records_path, "recs.jsonl");
        assert!(o.json && o.key_hex.is_some());
        // exige exactement une source de clé.
        assert!(Opts::parse(&["r.jsonl".into()]).is_err(), "clé manquante");
        assert!(Opts::parse(&["r.jsonl".into(), "--key-hex".into(), "x".into(), "--key-file".into(), "p".into()]).is_err(), "deux clés");
        assert!(Opts::parse(&["--key-hex".into(), "x".into()]).is_err(), "records manquant");
        // Ne vole pas l'option suivante comme valeur : `--root --json` est une erreur d'usage, pas un
        // faux verdict d'altération.
        assert!(Opts::parse(&["r.jsonl".into(), "--root".into(), "--json".into()]).is_err(), "flag-comme-valeur refusé");
        assert!(Opts::parse(&["r.jsonl".into(), "--key-hex".into(), "--json".into()]).is_err(), "clé-comme-flag refusée");
    }

    #[test]
    fn empty_bundle_is_refused_not_attested() {
        // Régression : un bundle vide / blanc ne doit PAS rendre « VÉRIFIÉ » exit 0 (la chaîne vide
        // re-prouve trivialement, clé jamais consultée). On le refuse, quelle que soit la clé.
        for body in ["", "\n\n   \n"] {
            let v = run(body, &key(), None).unwrap();
            assert!(!v.ok, "bundle vide refusé ({body:?})");
            assert_eq!(v.n_records, 0);
            assert!(v.reason.as_deref().unwrap_or_default().contains("vide"));
        }
        // Et via le binaire complet : exit code 1 (refus), pas 0.
        let path = std::env::temp_dir().join("prov-verify-empty.jsonl");
        std::fs::write(&path, "").unwrap();
        let code = real_main(&["prog".into(), path.to_string_lossy().to_string(), "--key-hex".into(), key_hex_of()]);
        assert_eq!(code, 1, "bundle vide → exit 1, jamais 0");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn json_output_is_well_formed_and_escaped() {
        let (jsonl, _) = sample_jsonl();
        let v = run(&jsonl, &key(), None).unwrap();
        let j = verdict_json(&v);
        assert!(j.starts_with("{\"ok\":true,"), "{j}");
        assert!(j.contains("\"manifest_root\":\""));
        // un motif d'échec avec guillemets s'échappe proprement.
        let tampered = run(&jsonl.replace("PASS", "FAIL"), &key(), None).unwrap();
        let jf = verdict_json(&tampered);
        assert!(jf.contains("\"ok\":false"));
        // pas de guillemet nu non-échappé qui casserait le JSON (heuristique simple).
        assert_eq!(jf.matches('"').count() % 2, 0, "guillemets équilibrés : {jf}");
    }

    #[test]
    fn real_main_exit_codes() {
        // Écrit un JSONL temporaire, vérifie les codes de sortie 0 / 1 / 2.
        let (jsonl, _) = sample_jsonl();
        let dir = std::env::temp_dir();
        let ok_path = dir.join("prov-verify-ok.jsonl");
        std::fs::write(&ok_path, &jsonl).unwrap();
        let kh = key_hex_of();
        let ok = real_main(&[
            "prog".into(), ok_path.to_string_lossy().to_string(), "--key-hex".into(), kh.clone(),
        ]);
        assert_eq!(ok, 0, "DAG authentique → 0");

        let bad_path = dir.join("prov-verify-bad.jsonl");
        std::fs::write(&bad_path, jsonl.replace("PASS", "FAIL")).unwrap();
        let bad = real_main(&["prog".into(), bad_path.to_string_lossy().to_string(), "--key-hex".into(), kh]);
        assert_eq!(bad, 1, "DAG altéré → 1");

        let usage = real_main(&["prog".into()]);
        assert_eq!(usage, 2, "usage incomplet → 2");

        let _ = std::fs::remove_file(&ok_path);
        let _ = std::fs::remove_file(&bad_path);
    }
}
