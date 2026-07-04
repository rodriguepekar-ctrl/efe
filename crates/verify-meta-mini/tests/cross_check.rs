// SPDX-License-Identifier: UNLICENSED
//! **Le cross-check de FIDÉLITÉ** — la preuve que `verify-meta-mini` (zéro-dép, tout fait main) calcule
//! EXACTEMENT ce qu'`efe-audit-core` scelle et ce qu'`efe-meta-verify` vérifie. Deux preuves, sans que le
//! binaire release ne dépende de rien (les deux crates sont en **dev-dep** uniquement).

use verify_meta_mini::verify::{from_hex32, seal_step, to_hex, verify, Outcome, Step};

/// 1) VECTEURS GOLDEN repris d'`efe-audit-core::hmac` — figent la construction du maillon.
/// `chain_hmac(parent, payload) = HMAC-SHA256(clé, parent‖payload)`, `parent = b"genesis"` à la genèse.
#[test]
fn golden_chain_hmac_matches_audit_core() {
    let key = b"test-master-32-bytes-fixed-aaaaaaaaaaaa";
    let payload = [0xABu8; 32];
    assert_eq!(
        to_hex(&seal_step(None, &payload, key)),
        "583a95c2cb8af0231bc58e4bea09891bc8f5f2f595891f642e5c315fa87ae907",
        "genèse — identique au golden efe-audit-core"
    );
    assert_eq!(
        to_hex(&seal_step(Some(&[0x11u8; 32]), &payload, key)),
        "ccebc3af4684e39038f0cd79c52b24b03cffb7a59d1f1c4b4e75b7ba11692c89",
        "parent 0x11..11 — identique au golden efe-audit-core"
    );
}

/// 2) ROUND-TRIP contre `efe-meta-verify` (la référence P1.1) : sceller là-bas, sérialiser en JSONL,
/// puis re-parser + re-prouver ici → concordance octet à octet, et altération localisée.
#[test]
fn roundtrip_against_efe_meta_verify() {
    use efe_meta_verify::{seal_in_place, SealedMetaStep};

    let key = [0x2au8; 32];
    let mut reference: Vec<SealedMetaStep> = (0..6u64)
        .map(|i| SealedMetaStep {
            meta_id: "kovant-procurement-v1".into(),
            step: i,
            residual: 7.4960402424643995 / (i as f64 + 1.0),
            consensus: vec![f64::from(-0.190_753_86_f32), f64::from(0.328_241_91_f32), f64::from(0.554_084_1_f32), i as f64],
            seal: String::new(),
        })
        .collect();
    let root_hex = seal_in_place(&mut reference, &key);
    let expected_root = from_hex32(&root_hex).unwrap();

    // Sérialiser comme l'émetteur (serde_json), puis re-parser avec le parseur FAIT MAIN de verify-meta-mini.
    let mini: Vec<Step> = reference
        .iter()
        .map(|s| Step::from_json_line(&serde_json::to_string(s).unwrap()).unwrap())
        .collect();

    match verify(&mini, &key, &expected_root) {
        Outcome::Ok { root } => assert_eq!(to_hex(&root), root_hex, "root re-calculé identique à la référence P1.1"),
        Outcome::Broken { at, reason } => panic!("verify-meta-mini diverge d'efe-meta-verify au pas {at} : {reason}"),
    }

    // Altération d'un octet de contenu → cassé au bon pas.
    let mut tampered = mini;
    tampered[3].consensus[0] += 1.0;
    assert!(
        matches!(verify(&tampered, &key, &expected_root), Outcome::Broken { at: 3, .. }),
        "altération localisée au pas 3"
    );

    // Mauvaise clé → cassé au pas 0.
    assert!(
        matches!(verify(&tampered_reference_reparsed(&reference), &[9u8; 32], &expected_root), Outcome::Broken { at: 0, .. }),
        "mauvaise clé → rupture au pas 0"
    );
}

fn tampered_reference_reparsed(reference: &[efe_meta_verify::SealedMetaStep]) -> Vec<Step> {
    reference.iter().map(|s| Step::from_json_line(&serde_json::to_string(s).unwrap()).unwrap()).collect()
}
