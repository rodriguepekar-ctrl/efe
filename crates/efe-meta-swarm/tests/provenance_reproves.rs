// SPDX-License-Identifier: UNLICENSED
//! **Le câblage moteur, prouvé** (feature `provenance`) : la provenance scellée PAR le moteur
//! (`iterate_with_provenance`) (1) a un pas par itération, (2) **se re-prouve** via `efe-meta-verify`
//! (le même bundle que P1.1/P1.2), (3) est **non-interférente** (mêmes numériques qu'`iterate`), et
//! (4) respecte la **souveraineté** (la clé n'apparaît nulle part dans le bundle sérialisé).
//!
//! **v4** (commit `4d54a9b`) : et, quand on la lui donne, (5) le moteur **scelle la liaison de run**
//! — qui a contribué, dans quel ORDRE, sous quelle configuration déclarée, sous quelle règle
//! d'agrégation. Le chemin `Some(...)` d'`iterate_with_provenance` n'était exercé **nulle part** :
//! `efe-meta-verify` prouve la liaison sur un `MetaChainState` piloté à la main, et le serveur la
//! prouve sur le bundle servi, mais entre les deux le sink du moteur (`new_bound`) n'avait aucun
//! test. Les cas ci-dessous ferment ce trou-là, et disent **laquelle des deux moitiés** de la
//! re-preuve attrape quelle falsification.

#![cfg(feature = "provenance")]

use efe_meta_swarm::prelude::*;
use efe_meta_swarm::provenance::{HmacKeySource, MetaProvenance, RunBinding, TenantId};

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
        // `None` — ce test n'a JAMAIS exercé de liaison de run, et les deux appels doivent
        // recevoir la même valeur : l'assertion (6) compare les deux roots pour prouver le
        // déterminisme, et un binding présent d'un côté seulement les ferait diverger pour une
        // raison qui n'a rien à voir avec ce que le test mesure.
        .iterate_with_provenance(&HmacKeySource::test_key(), TenantId::system(), None)
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
        // `None` — ce test n'a JAMAIS exercé de liaison de run, et les deux appels doivent
        // recevoir la même valeur : l'assertion (6) compare les deux roots pour prouver le
        // déterminisme, et un binding présent d'un côté seulement les ferait diverger pour une
        // raison qui n'a rien à voir avec ce que le test mesure.
        .iterate_with_provenance(&HmacKeySource::test_key(), TenantId::system(), None)
        .unwrap();
    assert_eq!(prov2.root, prov.root, "déterministe ⇒ le replay reproduit le root");
    assert_eq!(prov2.steps[1].vote_hashes, prov.steps[1].vote_hashes, "et les hashes de contribution");
}

// =====================================================================
// v4 — LE REÇU NOMME SES ACTEURS ET SA RÈGLE, côté MOTEUR
//
// Deux moitiés, à ne pas confondre — c'est ce que ces tests figent :
//   * `verify`        recompute la CHAÎNE depuis le condensé scellé dans chaque pas. Il ne lit
//                     JAMAIS la liaison du manifeste. Une identité réécrite là le laisse de marbre.
//   * `check_binding` recoupe la liaison PORTÉE par le manifeste avec celle SCELLÉE. C'est lui,
//                     et lui seul, qui attrape un fichier livré dont on a réécrit les noms.
// Un pipeline qui n'appellerait que le premier accepterait des identités falsifiées sous un root
// parfaitement valide. Les tests l'affirment POSITIVEMENT plutôt que de le taire.
//
// Ce que ces tests ne prouvent PAS, et qu'il ne faut pas leur faire dire : α/γ scellés sont ceux
// que l'APPELANT déclare, et le moteur ne les recoupe pas avec ceux qu'il applique. Sceller
// α=0.9 sur un run mené à α=0.5 passerait ici. Le seul garde-fou actuel est topologique — côté
// servi, `build_run_binding` et `wired_meta_operator` sont à quelques lignes l'un de l'autre.
// =====================================================================

/// La liaison du run de `build()` — les identifiants **réels** des deux participants, dans l'ordre
/// où le builder les a ajoutés, et les poids **réellement** câblés (α = 0.5, γ = 0.2).
///
/// Ce ne sont pas des étiquettes de test prises au hasard : une liaison qui nommerait « p0 »/« p1 »
/// scellerait des noms que personne ne pourrait recouper avec ce run, et le test ne prouverait plus
/// que l'encodage. Un participant déclare une configuration, l'autre est un mock qui n'en a aucune
/// à attester — les deux états dans la même liaison, pour que le marqueur de présence soit exercé.
fn binding() -> RunBinding {
    RunBinding::try_new(vec!["a".into(), "b".into()], vec![CFG_A.repeat(32), String::new()], "uniform", 0.5, 0.2).expect("liaison bien formée")
}

/// Le motif d'un `config_hash` déclaré (répété 32 fois = 64 hex).
const CFG_A: &str = "a1";

/// La clé tenant du run (dérivée, jamais dans le bundle).
fn key() -> Vec<u8> {
    HmacKeySource::test_key().derive(TenantId::system()).unwrap()
}

/// Un run scellé par le MOTEUR, avec ou sans liaison. Le déterminisme du moteur est établi par
/// l'assertion (6) du test ci-dessus : deux appels ne diffèrent que par ce qu'on change ici, donc
/// une différence de root s'impute à la liaison et à rien d'autre.
fn sealed_with(b: Option<RunBinding>) -> MetaProvenance {
    build().iterate_with_provenance(&HmacKeySource::test_key(), TenantId::system(), b).unwrap().1
}

/// **(5) — le moteur scelle la liaison dans CHAQUE pas, et le bundle se re-prouve.**
///
/// La liaison voyage **une fois** (dans l'artefact, puis le manifeste) mais son condensé est scellé
/// **partout** : c'est ce qui en fait un invariant de run, et non une annotation du premier pas
/// qu'un adversaire pourrait laisser derrière lui en tronquant.
#[test]
fn the_engine_seals_the_binding_into_every_step_and_the_bundle_reproves() {
    let prov = sealed_with(Some(binding()));

    assert_eq!(prov.binding.as_ref(), Some(&binding()), "l'artefact moteur porte la liaison");
    let h = binding().hash_hex();
    assert!(prov.steps.iter().all(|s| s.binding_hash.as_deref() == Some(h.as_str())), "les {} pas scellent la MÊME liaison", prov.steps.len());
    assert_eq!(prov.manifest().schema_version, 4, "une liaison ⟹ schéma 4 (et non 3)");

    // Le manifeste rend les acteurs LISIBLES — c'est ce qu'un auditeur vient chercher.
    let m = prov.manifest();
    let b = m.binding().expect("le manifeste du moteur porte la liaison");
    let cfg_a = CFG_A.repeat(32);
    let pairs: Vec<_> = b.participants().collect();
    assert_eq!(pairs, [("a", Some(cfg_a.as_str())), ("b", None)], "les deux participants sont nommés, en ordre, et le mock se lit comme tel");

    // Les DEUX moitiés de la re-preuve, toutes deux exigées.
    assert!(efe_meta_verify::verify(&prov.steps, &key(), &prov.root).ok, "la chaîne se re-prouve");
    efe_meta_verify::check_binding(&prov.steps, Some(&b)).expect("la liaison du manifeste correspond à celle scellée");
}

/// **LE test qui a des dents.** Permuter deux identités — mêmes noms, même nombre, mêmes votes —
/// est refusé. Sans ce cas négatif, on n'aurait prouvé que « ça encode », pas « ça attrape ».
///
/// Le test dit aussi **qui** attrape : pas `verify`. C'est délibérément affirmé au positif, parce
/// que cette répartition est la chose la plus facile à casser par inadvertance dans ce module.
#[test]
fn permuting_two_identities_is_refused_and_verify_alone_would_not_notice() {
    let prov = sealed_with(Some(binding()));

    // Le manifeste tel qu'il PART chez l'auditeur, falsifié sur le fil : deux identités permutées,
    // les pas scellés laissés intacts à l'octet près — l'adversaire n'a pas la clé tenant, donc il
    // ne peut toucher qu'au fichier qui n'est pas scellé.
    let mut forged: efe_meta_verify::MetaManifest = serde_json::from_str(&prov.manifest_json()).expect("manifeste relisible");
    forged.binding_participant_ids.as_mut().expect("v4 ⟹ liaison présente").swap(0, 1);
    forged.binding_declared_config_hashes.as_mut().expect("v4 ⟹ liaison présente").swap(0, 1);
    let on_the_wire = serde_json::to_string(&forged).expect("manifeste sérialisable");
    assert!(on_the_wire.contains(r#"["b","a"]"#), "la falsification est bien DANS le fichier livré : {on_the_wire}");
    let forged: efe_meta_verify::MetaManifest = serde_json::from_str(&on_the_wire).expect("manifeste falsifié relisible");

    // (a) `verify` seul ne voit RIEN — et c'est correct : la chaîne EST intègre, la falsification
    //     est ailleurs. Cette assertion tomberait si quelqu'un « réparait » `verify` en lui faisant
    //     lire le manifeste ; ce test devrait alors être réécrit sciemment, pas contourné.
    assert!(efe_meta_verify::verify(&prov.steps, &key(), &prov.root).ok, "la CHAÎNE reste intègre : rien n'a été touché dans les pas");

    // (b) `check_binding` l'attrape — c'est LUI la moitié qui nomme.
    let err = efe_meta_verify::check_binding(&prov.steps, forged.binding().as_ref()).expect_err("une permutation d'identités doit être refusée");
    assert!(err.contains("falsifiée"), "message inattendu : {err}");

    // (c) permuter les SEULS identifiants (config_hash laissés en place) est refusé aussi : la
    //     faute n'est pas « les colonnes ne concordent plus », c'est l'ordre lui-même.
    let mut ids_only = binding();
    ids_only.participant_ids.swap(0, 1);
    efe_meta_verify::check_binding(&prov.steps, Some(&ids_only)).expect_err("permuter les seuls identifiants doit être refusé");

    // (d) et l'adversaire ne peut pas non plus RE-SCELLER le run sous ses noms permutés pour faire
    //     concorder les deux fichiers : la liaison entre DANS le sceau, donc le root livré ne suit
    //     pas. (Sans la clé tenant il ne scelle rien du tout — ceci vaut même AVEC elle.)
    assert_ne!(sealed_with(Some(ids_only)).root, prov.root, "permuter les identités change le root : la liaison est DANS le sceau");
}

/// **Un `declared_config_hashes` réécrit casse la re-preuve.** Ce champ atteste la configuration
/// *demandée* ; la falsifier revient à dire « ce run a tourné sous une autre version ».
#[test]
fn rewriting_a_declared_config_hash_is_refused() {
    let prov = sealed_with(Some(binding()));

    let mut other_cfg = binding();
    other_cfg.declared_config_hashes[0] = "b2".repeat(32);
    let err = efe_meta_verify::check_binding(&prov.steps, Some(&other_cfg)).expect_err("réécrire un config_hash doit être refusé");
    assert!(err.contains("falsifiée"), "message inattendu : {err}");
    assert_ne!(sealed_with(Some(other_cfg)).root, prov.root, "la configuration déclarée est DANS le sceau");

    // Le cas vicieux : EFFACER la configuration du participant configuré pour le faire passer pour
    // un mock. « Absent » et « déclaré » sont deux états distincts, et tous deux scellés — c'est ce
    // que le marqueur de présence de l'encodage canonique existe pour tenir.
    let mut erased = binding();
    erased.declared_config_hashes[0] = String::new();
    efe_meta_verify::check_binding(&prov.steps, Some(&erased)).expect_err("effacer une configuration déclarée doit être refusé");
}

/// **La règle d'agrégation et ses DEUX poids participent au sceau.** Sans cela, un opérateur
/// pourrait changer d'agrégateur — ou de α, ou de γ — entre deux rondes sans que le reçu en garde
/// trace. γ n'était vérifié nulle part jusqu'ici, alors qu'il pèse autant que α dans le pas couplé.
#[test]
fn the_aggregation_rule_and_both_its_weights_are_sealed_too() {
    let prov = sealed_with(Some(binding()));

    let mut other_rule = binding();
    other_rule.operator = "attention".into();
    let mut other_alpha = binding();
    other_alpha.alpha = 0.6;
    let mut other_gamma = binding();
    other_gamma.gamma = 0.3;

    for (quoi, forged) in [("la règle", other_rule), ("α", other_alpha), ("γ", other_gamma)] {
        let err = efe_meta_verify::check_binding(&prov.steps, Some(&forged)).expect_err("changer la règle ou un poids doit être refusé");
        assert!(err.contains("falsifiée"), "changer {quoi} : message inattendu — {err}");
        assert_ne!(sealed_with(Some(forged)).root, prov.root, "changer {quoi} change le root");
    }
}

/// **Rétro-compatibilité, côté moteur.** Sans liaison, le moteur produit exactement le bundle v3
/// d'avant — c'est ce qui rend v4 additif et non cassant pour les reçus déjà émis.
#[test]
fn without_a_binding_the_engine_still_produces_the_v3_bundle() {
    let v3 = sealed_with(None);
    assert_eq!(v3.manifest().schema_version, 3, "sans liaison ⟹ schéma 3");
    assert!(v3.binding.is_none() && v3.steps.iter().all(|s| s.binding_hash.is_none()), "aucun octet de liaison dans un bundle v3");
    assert!(efe_meta_verify::verify(&v3.steps, &key(), &v3.root).ok, "un bundle v3 se re-prouve à l'identique");

    // Un bundle v3 n'a rien à recouper, et le contrôle doit l'ACCEPTER : le refuser reviendrait à
    // invalider rétroactivement tous les reçus antérieurs à v4.
    efe_meta_verify::check_binding(&v3.steps, None).expect("un bundle v3 n'a aucune liaison à recouper");
    // Mais coller un manifeste v4 sur des pas v3 — une promotion abusive, « ce vieux run était en
    // fait nominatif » — est refusé.
    efe_meta_verify::check_binding(&v3.steps, Some(&binding())).expect_err("un manifeste v4 sur des pas v3 doit être refusé");

    assert_ne!(sealed_with(Some(binding())).root, v3.root, "la liaison entre dans le sceau : le root v4 diffère du v3");
}
