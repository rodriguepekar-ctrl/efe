// SPDX-License-Identifier: UNLICENSED
//! **Le câblage moteur, prouvé** (feature `provenance`) : la provenance scellée PAR le moteur
//! (`iterate_with_provenance`) (1) a un pas par itération, (2) **se re-prouve** via `efe-meta-verify`
//! (le même bundle que P1.1/P1.2), (3) est **non-interférente** (mêmes numériques qu'`iterate`), et
//! (4) respecte la **souveraineté** (la clé n'apparaît nulle part dans le bundle sérialisé).

#![cfg(feature = "provenance")]

use efe_meta_swarm::prelude::*;
use efe_meta_swarm::provenance::{HmacKeySource, TenantId};

fn build() -> MetaSwarm {
    MetaSwarm::builder()
        .id("test-meta")
        .meta_space_dim(4)
        .add_participant(MockParticipant::with_contraction("a", 8, 0.6, 1).unwrap().boxed())
        .add_participant(MockParticipant::with_contraction("b", 8, 0.5, 2).unwrap().boxed())
        .lift_for("a", LinearLift::identity(8, 4).unwrap().boxed())
        .lift_for("b", LinearLift::identity(8, 4).unwrap().boxed())
        .pull_for("a", LinearPull::identity(4, 8).unwrap().boxed())
        .pull_for("b", LinearPull::identity(4, 8).unwrap().boxed())
        .meta_operator(UniformMetaOp::new(4).boxed())
        .meta_safety(BoxSafety::symmetric(4, 10.0).boxed())
        .alpha_default(0.5)
        .coupling_gamma(0.2)
        .tolerance(1e-6)
        .max_iters(200)
        .build()
        .unwrap()
}

#[test]
fn engine_provenance_reproves_matches_iterate_and_hides_the_key() {
    let (result, prov) = build()
        .iterate_with_provenance(&HmacKeySource::test_key(), TenantId::system())
        .unwrap();

    // (1) un pas scellé par itération.
    assert_eq!(prov.steps.len(), result.iterations, "un pas scellé par itération");
    assert!(prov.steps.len() >= 2, "quelques pas au moins");

    // (2) le bundle scellé PAR le moteur se re-prouve avec le vérifieur P1.1 (même format que P1.2).
    let key = HmacKeySource::test_key().derive(TenantId::system()).unwrap();
    let verdict = efe_meta_verify::verify(&prov.steps, &key, &prov.root);
    assert!(verdict.ok, "le bundle moteur se re-prouve (broken_at={:?})", verdict.broken_at);

    // altération → cassé au bon pas.
    let mut tampered = prov.steps.clone();
    let k = tampered.len() / 2;
    tampered[k].consensus[0] += 1.0;
    let bad = efe_meta_verify::verify(&tampered, &key, &prov.root);
    assert!(!bad.ok && bad.broken_at == Some(k), "altération localisée au pas {k}");

    // (3) non-interférence : iterate() seul donne les mêmes numériques (instance fraîche, mêmes seeds).
    let plain = build().iterate().unwrap();
    assert_eq!(plain.iterations, result.iterations, "mêmes itérations");
    assert_eq!(plain.final_residual.to_bits(), result.final_residual.to_bits(), "même résidu final");
    assert_eq!(plain.meta_consensus.as_slice(), result.meta_consensus.as_slice(), "même consensus");

    // (4) souveraineté : la clé n'apparaît nulle part dans le bundle sérialisé.
    let key_hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    assert!(!prov.steps_jsonl().contains(&key_hex), "clé absente du meta_steps.jsonl");
    assert!(!prov.root.contains(&key_hex), "clé absente du root");
    assert_eq!(prov.manifest().n_steps, prov.steps.len());

    // (5) v2 + v3 : le moteur scelle l'enveloppe de sûreté ET les contributions → schéma v3.
    assert_eq!(prov.manifest().schema_version, 3, "le moteur produit du v3 (safety_delta + vote_hashes)");
    assert!(prov.steps.iter().all(|s| s.safety_delta.is_some()), "chaque pas porte un safety_delta");
    assert!(prov.steps.iter().all(|s| s.safety_delta.unwrap() >= 0.0), "safety_delta = ‖·‖ ≥ 0");
    // v3 : un hash de contribution par participant (2 ici), en ordre.
    assert!(prov.steps.iter().all(|s| s.vote_hashes.as_ref().is_some_and(|h| h.len() == 2)), "2 vote_hashes/pas (2 participants)");

    // (6) REPLAY-REPRODUCTIBILITÉ : re-exécuter (mêmes seeds, même clé) reproduit le MÊME root et les
    // mêmes hashes de contribution — la provenance scellée correspond à une exécution déterministe.
    let (_r2, prov2) = build()
        .iterate_with_provenance(&HmacKeySource::test_key(), TenantId::system())
        .unwrap();
    assert_eq!(prov2.root, prov.root, "déterministe ⇒ le replay reproduit le root");
    assert_eq!(prov2.steps[1].vote_hashes, prov.steps[1].vote_hashes, "et les hashes de contribution");
}
