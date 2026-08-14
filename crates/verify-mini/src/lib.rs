// SPDX-License-Identifier: UNLICENSED
//! # `verify-mini` — Provenance P1 / S4 : le vérifieur forensic à arbre de dépendances **LITTÉRALEMENT VIDE**
//!
//! Le raffinement « **trust-only-the-math, sans astérisque** » du fossé (S4). `provenance-verify` ne
//! dépend que de `provenance-dag`, mais celui-ci tire `serde`/`serde_json` pour le JSONL — une dépendance
//! standard, mais une dépendance. `verify-mini` la **supprime** : il **embarque** sa propre SHA-256
//! ([`sha256`], FIPS 180-4) et HMAC ([`hmac`], RFC 2104), **parse le JSONL à la main** ([`json`], pas de
//! serde), et **ré-implémente l'encodage canonique** ([`record`]) et la re-preuve ([`verify`]). Résultat :
//! `cargo tree` (edges normales) ne montre que `verify-mini` — un tiers le re-compile en ne faisant
//! confiance qu'aux **maths** et à sa **propre compilation**, sans rien d'autre à auditer.
//!
//! ## Fidélité à S0, prouvée (pas affirmée)
//! Un vérifieur n'a de valeur que s'il calcule **exactement** ce que S0 a scellé. C'est garanti par un
//! **cross-check** (`tests/cross_check_against_s0.rs`) : on construit des DAG avec `provenance-dag`
//! (en **dev-dependency** uniquement — hors du binaire release), et on exige que `verify-mini` rende le
//! **manifest-root identique octet pour octet**, accepte l'authentique, et rejette l'altéré. Les chaînes
//! piégées (guillemets, antislash, contrôles, unicode, paires de substitution) y sont incluses — c'est là
//! que le parseur fait main pourrait diverger, et c'est là qu'on le verrouille.
//!
//! ## Contrat CLI (identique à `provenance-verify`)
//! ```text
//!   verify-mini <records.jsonl> (--key-hex <64hex> | --key-file <chemin>) [--root <attendu>] [--json]
//! ```
//! Codes : `0` = vérifié intègre · `1` = échec (altéré/réordonné/mauvaise clé/root) · `2` = usage/E-S.
//! **Sémantique** : atteste l'**INTÉGRITÉ** — pas la correction, pas une reproductibilité bit-à-bit.
//!
//! ## Usage (la branche d'échec, sans infra de scellement)
//! ```
//! // Un record dont le sceau « 00 » est forgé : verify-mini le refuse (sans rien d'externe).
//! let jsonl = r#"{"kind":"agent_act","node_id":"c::a","agent_role":"r","inputs_hash":"h","model":"m","version":"1","output":"PASS","evidence_refs":[],"timestamp":1,"seal":"00"}"#;
//! let key = [7u8; 32];
//! let verdict = verify_mini::run(jsonl, &key, None).unwrap();
//! assert!(!verdict.ok, "un sceau forgé ne re-prouve pas");
//! assert_eq!(verdict.broken_at, Some(0));
//! ```

pub mod hmac;
pub mod json;
pub mod record;
pub mod sha256;
pub mod verify;

pub use record::{MiniNode, MiniRecord};
pub use verify::{reprove, verify_against_root, Verdict, GENESIS};

/// Le point d'entrée de haut niveau : parse le JSONL, **refuse un bundle vide** (une chaîne vide
/// re-prouverait trivialement — la clé jamais consultée — ce qui serait un faux positif), puis re-prouve
/// (ou vérifie contre un root publié). `key` est les **32 octets bruts** de la clé tenant.
pub fn run(records_jsonl: &str, key: &[u8], expected_root: Option<&str>) -> Result<Verdict, String> {
    let records = MiniRecord::parse_jsonl(records_jsonl).map_err(|e| format!("JSONL invalide : {e}"))?;
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
        None => reprove(&records, key),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bundle_is_refused() {
        for body in ["", "\n\n  \n"] {
            let v = run(body, &[7u8; 32], None).unwrap();
            assert!(!v.ok && v.n_records == 0);
            assert!(v.reason.as_deref().unwrap_or_default().contains("vide"));
        }
    }

    #[test]
    fn bad_jsonl_is_an_error() {
        assert!(run("ceci n'est pas du json", &[7u8; 32], None).is_err());
    }
}
