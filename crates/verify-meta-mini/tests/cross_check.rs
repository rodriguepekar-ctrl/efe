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
            safety_delta: None,
            vote_hashes: None,
            binding_hash: None,
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

/// 3) **v2** : un bundle scellé AVEC `safety_delta` (l'enveloppe de sûreté) se re-prouve aussi ici, et le
/// delta est bien re-parsé par le parseur fait main.
#[test]
fn roundtrip_v2_safety_delta() {
    use efe_meta_verify::MetaChainState;

    let key = [0x2au8; 32];
    let mut chain = MetaChainState::new("kovant-procurement-v1", key.to_vec());
    chain.push(0, 1.0, vec![0.1, 0.2, 0.3], Some(0.0), None); // enveloppe inactive
    chain.push(1, 0.4, vec![0.4, 0.5, 0.6], Some(0.47), None); // enveloppe ACTIVE
    chain.push(2, 0.01, vec![0.7, 0.8, 0.9], Some(0.0), None);
    let prov = chain.finish();
    let expected_root = from_hex32(&prov.root).unwrap();

    let mini: Vec<Step> = prov
        .steps
        .iter()
        .map(|s| Step::from_json_line(&serde_json::to_string(s).unwrap()).unwrap())
        .collect();

    assert_eq!(mini[1].safety_delta, Some(0.47), "le safety_delta est re-parsé (parseur fait main)");
    match verify(&mini, &key, &expected_root) {
        Outcome::Ok { root } => assert_eq!(to_hex(&root), prov.root, "v2 : root identique à la référence"),
        Outcome::Broken { at, reason } => panic!("verify-meta-mini diverge d'efe-meta-verify (v2) au pas {at} : {reason}"),
    }
}

/// 4) **v3** : un bundle avec `vote_hashes` (contributions participants, tableau de chaînes) se re-prouve
/// aussi ici — le parseur fait main lit le tableau de chaînes et le hash entre dans le canonique.
#[test]
fn roundtrip_v3_vote_hashes() {
    use efe_meta_verify::{content_hash_hex, MetaChainState};

    let key = [0x2au8; 32];
    let mut chain = MetaChainState::new("kovant-procurement-v1", key.to_vec());
    let vh = |a: &[f64], b: &[f64]| Some(vec![content_hash_hex(a), content_hash_hex(b)]);
    chain.push(0, 1.0, vec![0.1, 0.2, 0.3], Some(0.0), vh(&[1.0, 2.0], &[3.0, 4.0]));
    chain.push(1, 0.01, vec![0.4, 0.5, 0.6], Some(1.65), vh(&[5.0, 6.0], &[7.0, 8.0]));
    let prov = chain.finish();
    let expected_root = from_hex32(&prov.root).unwrap();

    let mini: Vec<Step> = prov
        .steps
        .iter()
        .map(|s| Step::from_json_line(&serde_json::to_string(s).unwrap()).unwrap())
        .collect();

    // Le tableau de vote_hashes est re-parsé par le parseur fait main.
    assert_eq!(mini[1].vote_hashes.as_ref().unwrap().len(), 2, "2 vote_hashes re-parsés");
    assert_eq!(mini[1].vote_hashes, prov.steps[1].vote_hashes);
    match verify(&mini, &key, &expected_root) {
        Outcome::Ok { root } => assert_eq!(to_hex(&root), prov.root, "v3 : root identique à la référence"),
        Outcome::Broken { at, reason } => panic!("verify-meta-mini diverge d'efe-meta-verify (v3) au pas {at} : {reason}"),
    }
}
