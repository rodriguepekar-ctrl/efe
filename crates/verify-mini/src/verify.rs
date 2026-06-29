// SPDX-License-Identifier: UNLICENSED
//! **La re-preuve** — ré-implémentation fidèle de `provenance-dag::dag` (re-preuve + vérification contre un
//! root), sans serde. Chaque maillon : `sceau_i = HMAC(K, chaîne_{i-1} ‖ canonical(nœud_i))`,
//! `chaîne_0 = SHA256(GENESIS)`, `manifest-root = hex(chaîne_n)`. Intégrité **structurelle** aussi
//! (références ⊆ ids antérieurs ; ids uniques). Sémantique : atteste l'**INTÉGRITÉ** — pas la correction.

use crate::hmac::{ct_eq, hmac_sha256};
use crate::record::MiniRecord;
use crate::sha256::sha256;
use std::collections::HashSet;

/// Constante de séparation de domaine — l'ancre de la chaîne (versionnée, **identique** à S0).
pub const GENESIS: &[u8] = b"efe-prov-dag:v1:genesis";

/// Le verdict d'une re-preuve — mêmes champs que `provenance-dag::Verdict`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub ok: bool,
    pub n_records: usize,
    pub manifest_root: Option<String>,
    pub broken_at: Option<usize>,
    pub reason: Option<String>,
}
impl Verdict {
    fn ok(n: usize, root: String) -> Verdict {
        Verdict { ok: true, n_records: n, manifest_root: Some(root), broken_at: None, reason: None }
    }
    fn fail(at: usize, reason: impl Into<String>) -> Verdict {
        Verdict { ok: false, n_records: 0, manifest_root: None, broken_at: Some(at), reason: Some(reason.into()) }
    }
    fn fail_global(n: usize, reason: impl Into<String>) -> Verdict {
        Verdict { ok: false, n_records: n, manifest_root: None, broken_at: None, reason: Some(reason.into()) }
    }
}

/// **LA RE-PREUVE** — re-dérive la chaîne depuis le genesis et la clé, vérifie chaque sceau (à temps
/// constant) ET l'intégrité structurelle. Ne dépend que des records bruts + la clé.
pub fn reprove(records: &[MiniRecord], key: &[u8]) -> Verdict {
    let mut chain = sha256(GENESIS);
    let mut ids: HashSet<&str> = HashSet::new();
    for (i, rec) in records.iter().enumerate() {
        for r in rec.node.refs() {
            if !ids.contains(r) {
                return Verdict::fail(i, format!("référence pendante vers « {r} » (nœud absent, postérieur, ou auto-référence)"));
            }
        }
        if !ids.insert(rec.node.id()) {
            return Verdict::fail(i, format!("id de nœud dupliqué « {} »", rec.node.id()));
        }
        let mut buf = chain.to_vec();
        buf.extend_from_slice(&rec.node.canonical());
        let expect = hmac_sha256(key, &buf);
        let stored = match from_hex(&rec.seal) {
            Some(b) => b,
            None => return Verdict::fail(i, "sceau hex invalide"),
        };
        if !ct_eq(&expect, &stored) {
            return Verdict::fail(i, "sceau invalide : record altéré, réordonné, ou mauvaise clé tenant");
        }
        chain = expect;
    }
    Verdict::ok(records.len(), to_hex(&chain))
}

/// Re-preuve **contre un manifest-root publié** : intégrité + root re-dérivé == root publié.
pub fn verify_against_root(records: &[MiniRecord], key: &[u8], expected_root: &str) -> Verdict {
    let v = reprove(records, key);
    if !v.ok {
        return v;
    }
    match &v.manifest_root {
        Some(root) if root == expected_root => v,
        other => Verdict::fail_global(
            records.len(),
            format!(
                "intégrité OK mais manifest-root publié NON concordant — attendu « {expected_root} », obtenu « {} » (records absents/ajoutés, ou mauvais bundle)",
                other.clone().unwrap_or_default()
            ),
        ),
    }
}

// ── hex (zéro dépendance, comme S0) ──

fn to_hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

/// Hex → octets. Strict : longueur paire ET uniquement des chiffres hexa (rejette « +a… » que
/// `from_str_radix` accepterait sinon — aliasing de chaînes distinctes vers le même octet).
fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Une clé fixe (les octets bruts d'une clé tenant — verify-mini les utilise tels quels en clé HMAC).
    const KEY: [u8; 32] = [7u8; 32];

    fn line(kind_json: &str) -> MiniRecord {
        MiniRecord::parse_line(kind_json).unwrap()
    }

    /// Construit un petit JSONL scellé AVEC verify-mini lui-même (cohérence interne) : on calcule les
    /// sceaux à la main via la même chaîne, puis on vérifie qu'ils re-prouvent. (Le cross-check contre S0
    /// — la VRAIE preuve de fidélité — est dans `tests/`.)
    fn sealed_pair() -> Vec<MiniRecord> {
        // On ne peut pas sceller facilement ici sans dupliquer la logique ; on s'appuie sur le cross-check
        // S0 (tests/). Ici on teste juste les branches d'ÉCHEC sur des records forgés.
        vec![line(r#"{"kind":"agent_act","node_id":"n1","agent_role":"r","inputs_hash":"h","model":"m","version":"1","output":"o","evidence_refs":[],"timestamp":1,"seal":"00"}"#)]
    }

    #[test]
    fn forged_seal_fails_at_record_zero() {
        let v = reprove(&sealed_pair(), &KEY);
        assert!(!v.ok && v.broken_at == Some(0), "un sceau forgé échoue au record 0");
    }

    #[test]
    fn dangling_reference_is_rejected() {
        let forged = vec![line(r#"{"kind":"edge","edge_id":"e","from_node":"ghost","to_node":"x","artifact_hash":"h","artifact_type":"t","timestamp":1,"seal":"00"}"#)];
        let v = reprove(&forged, &KEY);
        assert!(!v.ok && v.broken_at == Some(0));
        assert!(v.reason.as_deref().unwrap_or_default().contains("pendante"));
    }

    #[test]
    fn bad_seal_hex_is_caught() {
        let forged = vec![line(r#"{"kind":"agent_act","node_id":"a","agent_role":"r","inputs_hash":"h","model":"m","version":"1","output":"o","evidence_refs":[],"timestamp":1,"seal":"+a000"}"#)];
        let v = reprove(&forged, &KEY);
        assert!(!v.ok, "sceau « +a000 » non-canonique rejeté");
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(to_hex(&[0x00, 0xab, 0xff]), "00abff");
        assert_eq!(from_hex("00abff"), Some(vec![0x00, 0xab, 0xff]));
        assert_eq!(from_hex("0a0"), None, "longueur impaire");
        assert_eq!(from_hex("+a"), None, "non-canonique");
    }
}
