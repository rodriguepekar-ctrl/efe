// SPDX-License-Identifier: UNLICENSED
//! **Le cross-check de FIDÉLITÉ** — la preuve qu'un vérifieur sans serde calcule **exactement** ce que S0
//! a scellé. On construit des DAG avec `provenance-dag` (en **dev-dependency** — il n'entre PAS dans le
//! binaire release), on en prend le JSONL + le manifest-root, et on exige que `verify-mini` :
//! 1. rende le **manifest-root IDENTIQUE octet pour octet** (sinon le parseur fait main ou l'encodage
//!    canonique diverge — c'est le risque exact d'une ré-implémentation) ;
//! 2. **accepte** l'authentique, **contre le root publié** aussi ;
//! 3. **rejette** l'altéré (au bon record) et la **mauvaise clé**.
//!
//! Les chaînes **piégées** (guillemets, antislash, retour-ligne, contrôle, unicode, paire de substitution)
//! sont incluses **à dessein** : c'est là qu'un décodage JSON imparfait casserait la concordance.

use provenance_dag::{reprove_dag, HumanAction, Node, SealedDag, TenantKey};

/// Les octets bruts de la clé tenant — S0 les utilise comme clé HMAC ; verify-mini reçoit les mêmes octets.
const KEY: [u8; 32] = [7u8; 32];

/// Un DAG réaliste « conformité fournisseur » avec des chaînes PIÉGÉES dans les champs free-text.
fn rich_dag() -> SealedDag {
    let k = TenantKey::from_bytes(KEY);
    let mut d = SealedDag::new();
    d.append(&k, Node::AgentAct {
        node_id: "SUP-2024::extract".into(),
        agent_role: "supplier_extract".into(),
        inputs_hash: "h".into(),
        model: "phi4".into(),
        version: "1".into(),
        // PIÈGES : guillemets, antislash, accolades, café/unicode, emoji (paire de substitution), retour-ligne.
        output: "{\"supplier\":\"ACME\"} café — déjà \\chemin\\ 😀\nligne2".into(),
        evidence_refs: vec![],
        timestamp: 1,
    }).unwrap();
    d.append(&k, Node::Evidence {
        evidence_id: "SUP-2024::ev".into(),
        tool: "policy_db".into(),
        query_hash: "q".into(),
        retrieved_ref: "policy://4.2".into(),
        content_hash: "cc".into(),
        consumed_by_node: "SUP-2024::sanction".into(),
        timestamp: 2,
    }).unwrap();
    d.append(&k, Node::AgentAct {
        node_id: "SUP-2024::sanction".into(),
        agent_role: "sanction_check".into(),
        inputs_hash: "h".into(),
        model: "phi4".into(),
        version: "1".into(),
        output: "PASS".into(),
        evidence_refs: vec!["SUP-2024::ev".into()],
        timestamp: 3,
    }).unwrap();
    d.append(&k, Node::Edge {
        edge_id: "SUP-2024::e1".into(),
        from_node: "SUP-2024::extract".into(),
        to_node: "SUP-2024::sanction".into(),
        artifact_hash: "a".into(),
        artifact_type: "supplier_record".into(),
        timestamp: 4,
    }).unwrap();
    d.append(&k, Node::AgentAct {
        node_id: "SUP-2024::aggregate".into(),
        agent_role: "verdict_aggregate".into(),
        inputs_hash: "h".into(),
        model: "phi4".into(),
        version: "1".into(),
        output: "COMPLIANT".into(),
        evidence_refs: vec![],
        timestamp: 5,
    }).unwrap();
    d.append(&k, Node::Human {
        node_id: "SUP-2024::review".into(),
        human_id: "officer:hanna".into(),
        action: HumanAction::Override,
        target_node: "SUP-2024::aggregate".into(),
        // PIÈGES dans la justification : guillemets, tab, unicode.
        justification: "revu : \"faux positif\" — déjà\tcontrôlé".into(),
        timestamp: 6,
    }).unwrap();
    d
}

#[test]
fn verify_mini_reproduces_s0_manifest_root_byte_for_byte() {
    let d = rich_dag();
    let jsonl = d.to_jsonl();
    let s0_root = d.manifest_root();

    // verify-mini re-prouve et rend le MÊME root — c'est toute la fidélité, sur des chaînes piégées.
    let v = verify_mini::run(&jsonl, &KEY, None).expect("JSONL parsable par verify-mini");
    assert!(v.ok, "verify-mini accepte le DAG authentique : {:?}", v.reason);
    assert_eq!(v.n_records, 6);
    assert_eq!(v.manifest_root.as_deref(), Some(s0_root.as_str()), "manifest-root IDENTIQUE à S0 (octet pour octet)");

    // Et contre le root publié.
    assert!(verify_mini::run(&jsonl, &KEY, Some(&s0_root)).unwrap().ok, "concordance avec le root publié");
    // Un mauvais root publié → échec global (pas de broken_at).
    let wrong = verify_mini::run(&jsonl, &KEY, Some(&"ff".repeat(32))).unwrap();
    assert!(!wrong.ok && wrong.broken_at.is_none(), "root publié non concordant → échec global");
}

#[test]
fn verify_mini_agrees_with_s0_verdict() {
    // Double-verrou : la re-preuve de S0 et celle de verify-mini doivent rendre le MÊME verdict.
    let d = rich_dag();
    let jsonl = d.to_jsonl();
    let s0 = reprove_dag(d.records(), &TenantKey::from_bytes(KEY));
    let mini = verify_mini::run(&jsonl, &KEY, None).unwrap();
    assert_eq!(s0.ok, mini.ok);
    assert_eq!(s0.manifest_root, mini.manifest_root, "même root des deux côtés");
    assert_eq!(s0.n_records, mini.n_records);
}

#[test]
fn verify_mini_detects_tampering_at_the_right_record() {
    let d = rich_dag();
    let jsonl = d.to_jsonl();
    // Un falsificateur change le verdict d'agrégation (record 4) dans le JSONL brut.
    let tampered = jsonl.replacen("COMPLIANT", "NON_COMPLIANT", 1);
    let v = verify_mini::run(&tampered, &KEY, None).unwrap();
    assert!(!v.ok, "altération détectée");
    assert_eq!(v.broken_at, Some(4), "cassé exactement au record d'agrégation altéré");
    // S0 doit localiser au MÊME endroit (cohérence des deux vérifieurs).
    let recs = SealedDag::records_from_jsonl(&tampered).unwrap();
    assert_eq!(reprove_dag(&recs, &TenantKey::from_bytes(KEY)).broken_at, Some(4));
}

#[test]
fn verify_mini_rejects_a_wrong_key() {
    let d = rich_dag();
    let jsonl = d.to_jsonl();
    let v = verify_mini::run(&jsonl, &[9u8; 32], None).unwrap();
    assert!(!v.ok && v.broken_at == Some(0), "mauvaise clé → échec dès le 1er record");
}

#[test]
fn verify_mini_matches_s0_on_a_minimal_dag_too() {
    // Un DAG minimal (un seul acte) — la concordance ne dépend pas de la richesse.
    let k = TenantKey::from_bytes([3u8; 32]);
    let mut d = SealedDag::new();
    d.append(&k, Node::AgentAct { node_id: "n".into(), agent_role: "r".into(), inputs_hash: "h".into(), model: "m".into(), version: "1".into(), output: "x".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
    let v = verify_mini::run(&d.to_jsonl(), &[3u8; 32], None).unwrap();
    assert!(v.ok && v.manifest_root.as_deref() == Some(d.manifest_root().as_str()));
}
