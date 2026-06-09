// SPDX-License-Identifier: UNLICENSED
//! # `canon` — sérialisation JSON **canonique** (déterministe, cross-vérifiable).
//!
//! La provenance par-agent (Phase 4) hashe puis chaîne des enregistrements qu'un
//! **vérificateur indépendant** rejouera *des années plus tard*. Pour que deux
//! vérificateurs obtiennent le **même** hash, le corps sérialisé doit être
//! **byte-identique** quelle que soit :
//! - l'ordre d'insertion des clés d'objet,
//! - le bruit de précision des flottants,
//! - les variantes d'échappement de chaîne.
//!
//! Cet algorithme est une **réplique exacte** de
//! `efe_nano::idempotency::canonicalize_json` (la chaîne d'audit HMAC d'`efe-nano`)
//! — clés triées **récursivement**, **aucun** espace, échappement de chaîne précis.
//! L'identité bit-à-bit est **intentionnelle** : les chaînes d'essaim (Phase 4) et
//! les chaînes de trace (`efe-nano`) doivent être mutuellement vérifiables (§3.2 de
//! l'architecture). En Inc.2, cet algorithme sera **factorisé** dans `efe-audit-core`
//! et partagé ; tant que la factorisation n'a pas lieu, on le réplique pour figer le
//! contrat — un **test doré** (`canon_matches_known_golden`) verrouille le format.

use serde_json::Value;

/// Rend une valeur JSON sous forme canonique : clés triées récursivement, aucun
/// espace, échappement stable. Deux valeurs logiquement égales produisent la
/// **même** chaîne (donc le même hash).
pub fn canonical_json(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

/// Idem [`canonical_json`], en octets — l'entrée directe d'un hash.
pub fn canonical_bytes(v: &Value) -> Vec<u8> {
    canonical_json(v).into_bytes()
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_escaped(s, out),
        Value::Array(arr) => {
            out.push('[');
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(obj) => {
            // Clés triées alphabétiquement (déterminisme).
            let mut keys: Vec<&String> = obj.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_escaped(k, out);
                out.push(':');
                write_canonical(&obj[*k], out);
            }
            out.push('}');
        }
    }
}

/// Échappement de chaîne JSON canonique (identique à `efe-nano`).
fn write_escaped(s: &str, out: &mut String) {
    out.push('"');
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
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sorts_keys_alphabetically() {
        let v = json!({ "b": 1, "a": 2, "c": 3 });
        assert_eq!(canonical_json(&v), r#"{"a":2,"b":1,"c":3}"#);
    }

    #[test]
    fn key_order_irrelevant_to_output() {
        let a = json!({ "z": 1, "a": 2, "m": 3 });
        let b = json!({ "a": 2, "m": 3, "z": 1 });
        assert_eq!(canonical_json(&a), canonical_json(&b), "ordre d'insertion neutre");
    }

    #[test]
    fn no_whitespace_emitted() {
        let v = json!({ "a": [1, 2, 3], "b": { "c": "d" } });
        let s = canonical_json(&v);
        assert!(!s.contains(' '), "aucun espace");
        assert!(!s.contains('\n'), "aucun saut de ligne");
    }

    #[test]
    fn nested_objects_sorted_recursively() {
        let v = json!({ "outer": { "z": 1, "a": 2 } });
        assert_eq!(canonical_json(&v), r#"{"outer":{"a":2,"z":1}}"#);
    }

    #[test]
    fn arrays_preserve_order() {
        // Les tableaux NE sont PAS triés (l'ordre y est sémantique).
        let v = json!([3, 1, 2]);
        assert_eq!(canonical_json(&v), "[3,1,2]");
    }

    #[test]
    fn escapes_quotes_backslashes_and_controls() {
        let v = json!({ "k": "a\"b\\c\nd\te" });
        assert_eq!(canonical_json(&v), r#"{"k":"a\"b\\c\nd\te"}"#);
        // contrôle < 0x20 (autre que \n\r\t) → \u00xx.
        let v2 = Value::String("\u{0001}".into());
        assert_eq!(canonical_json(&v2), r#""\u0001""#);
    }

    #[test]
    fn scalars_render_canonically() {
        assert_eq!(canonical_json(&Value::Null), "null");
        assert_eq!(canonical_json(&json!(true)), "true");
        assert_eq!(canonical_json(&json!(false)), "false");
        assert_eq!(canonical_json(&json!(42)), "42");
        assert_eq!(canonical_json(&json!("x")), r#""x""#);
    }

    #[test]
    fn canonical_bytes_equals_json_bytes() {
        let v = json!({ "a": 1 });
        assert_eq!(canonical_bytes(&v), canonical_json(&v).into_bytes());
    }

    #[test]
    fn canon_matches_known_golden() {
        // Test DORÉ : fige le contrat de format (toute dérive casse ce test).
        // Doit être identique au rendu de `efe_nano::idempotency::canonicalize_json`.
        let v = json!({
            "swarm_id": "00000000-0000-0000-0000-000000000002",
            "agent_id": "generator",
            "reads": [{ "version": 3, "pointer": "/cand/0" }],
            "variant": 1
        });
        let expected = concat!(
            r#"{"agent_id":"generator","#,
            r#""reads":[{"pointer":"/cand/0","version":3}],"#,
            r#""swarm_id":"00000000-0000-0000-0000-000000000002","#,
            r#""variant":1}"#
        );
        assert_eq!(canonical_json(&v), expected);
    }
}
