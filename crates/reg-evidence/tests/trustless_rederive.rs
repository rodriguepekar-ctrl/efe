// SPDX-License-Identifier: UNLICENSED
//! **I-S6-VUE-REDÉRIVABLE / SUR-PROUVÉ / TRAÇABLE** (test d'intégration) — la propriété trust-light, de
//! bout en bout, sur le **vrai** chemin : l'export S2 (`ExportContent`) alimente la dérivation S6, puis,
//! **comme le ferait un tiers**, on **re-dérive** l'artefact à partir des **seuls** records + clé + mapping,
//! et on exige l'**égalité octet-à-octet**. On vérifie aussi que l'altération est **refusée** (jamais
//! d'évidence sur du non-prouvé) et que le **dossier fournisseur** CSDDD/LkSG (le ROI) se génère.
//!
//! C'est la matérialisation de « zéro confiance neuve » : S6 ne demande pas de le croire — sa sortie est
//! une **fonction déterministe** de données re-prouvées, que le tiers **re-calcule**.

use audit_api::{AuditService, AuthContext, HumanActionInput, ProjectionConfig, Role};
use provenance_capture::{LlmCall, MessageEvent, ToolCall};
use provenance_dag::{HumanAction, SealedDag, TenantKey};
use reg_evidence::{derive, reg_subtree, render_md, templates, CaseScope, Framework, MappingSet, RegError};

const KEY_BYTES: [u8; 32] = [21u8; 32];
fn key() -> TenantKey {
    TenantKey::from_bytes(KEY_BYTES)
}
fn officer() -> AuthContext {
    AuthContext::new("acme", Role::ComplianceOfficer, "officer:hanna")
}

/// Construit un cas « conformité fournisseur » complet via la capture S1 + le gate humain S2, puis rend
/// le JSONL re-prouvable de l'export S2 (le payload scellé complet du tenant).
fn build_via_s2() -> String {
    let mut svc = AuditService::new(key(), ProjectionConfig::default());
    // extraction (acte) ; évidence(politique) AVANT le contrôle qui la cite ; contrôle ; arête ; agrégation.
    svc.ingest_llm(&LlmCall { case_id: "SUP-2024", step: "extract", agent_role: "supplier_extract", inputs: b"dossier ACME", model: "phi4", version: "1", output: "{supplier:SUP-2024}", evidence_steps: &[], timestamp: 1 }).unwrap();
    svc.ingest_evidence(&ToolCall { case_id: "SUP-2024", step: "ev-pol", tool: "policy_db", query: b"sanctions?", retrieved_ref: "policy://4.2", content: b"clause 4.2 telle quau moment T", consumed_by_step: "sanction", timestamp: 2 }).unwrap();
    svc.ingest_llm(&LlmCall { case_id: "SUP-2024", step: "sanction", agent_role: "sanction_check", inputs: b"nom: ACME", model: "phi4", version: "1", output: "PASS", evidence_steps: &["ev-pol"], timestamp: 3 }).unwrap();
    svc.ingest_message(&MessageEvent { case_id: "SUP-2024", step: "e1", from_step: "extract", to_step: "sanction", artifact: b"supplier_record", artifact_type: "supplier_record", timestamp: 4 }).unwrap();
    svc.ingest_llm(&LlmCall { case_id: "SUP-2024", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"agg", model: "phi4", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 5 }).unwrap();
    // L'oversight humain scellé (le gate S2 — RBAC + cible validée).
    svc.record_human_action("SUP-2024", &HumanActionInput { target_step: "aggregate".into(), action: HumanAction::Approve, justification: "revue conforme".into() }, &officer(), 6).unwrap();
    // Un SECOND cas, pour vérifier que la portée par-cas n'en fuit rien.
    svc.ingest_llm(&LlmCall { case_id: "SUP-OTHER", step: "sanction", agent_role: "sanction_check", inputs: b"x", model: "phi4", version: "1", output: "PASS", evidence_steps: &[], timestamp: 7 }).unwrap();

    let exp = svc.export(None, &officer(), 100).expect("export S2");
    assert!(exp.integrity_ok, "l'export S2 est re-prouvé");
    exp.records_jsonl
}

#[test]
fn third_party_rederives_the_artifact_bit_for_bit() {
    let jsonl = build_via_s2();
    let mapping = templates::eu_ai_act_2026_06();
    let scope = CaseScope::Case("SUP-2024".into());

    // KOVANT produit l'artefact.
    let recs_a = SealedDag::records_from_jsonl(&jsonl).unwrap();
    let a = derive(&recs_a, &key(), &mapping, Framework::EuAiAct, &scope).expect("dérivation");

    // LE TIERS re-dérive depuis les SEULS records (re-parsés) + sa clé + le mapping embarqué — rien d'autre.
    let recs_b = SealedDag::records_from_jsonl(&jsonl).unwrap();
    let mapping_back: MappingSet = serde_json::from_str(&serde_json::to_string(&mapping).unwrap()).unwrap(); // comme s'il l'avait lu du bundle
    let b = derive(&recs_b, &key(), &mapping_back, Framework::EuAiAct, &scope).expect("re-dérivation tiers");

    // Égalité octet-à-octet (de l'artefact ET de la VUE rendue).
    assert_eq!(a, b, "artefact re-dérivé identique (trust-light : zéro confiance neuve)");
    assert_eq!(render_md(&a), render_md(&b), "récit re-dérivé identique");

    // Traçabilité : chaque id cité est présent dans les records re-prouvés (I-S6-TRAÇABLE).
    let present: std::collections::HashSet<&str> = recs_a.iter().map(|r| r.node.id()).collect();
    for f in &a.findings {
        for id in &f.evidence {
            assert!(present.contains(id.as_str()), "id cité présent : {id}");
            assert!(id.starts_with("SUP-2024::"), "aucune fuite d'un autre cas dans la portée par-cas : {id}");
        }
    }

    // Couverture honnête : l'EU AI Act déclare ses trous (I-S6-COUVERTURE-HONNÊTE visible dans l'artefact).
    assert!(a.coverage_summary.out_of_dag_scope >= 1, "des trous sont déclarés et comptés");
    assert!(a.findings.iter().any(|f| f.citation == "art. 9"), "le trou art. 9 est présent");

    // Le récit porte l'honnêteté, et n'affirme JAMAIS la conformité.
    let md = render_md(&a);
    assert!(md.to_lowercase().contains("outillé ≠ conforme"));
    assert!(!md.to_lowercase().contains("compliant"), "aucun verdict de conformité (le DAG contient pourtant un output « COMPLIANT », jamais cité comme évidence)");

    // L'art. 14 (oversight) cite le nœud humain scellé.
    let art14 = a.findings.iter().find(|f| f.citation == "art. 14").unwrap();
    assert!(art14.evidence.iter().any(|id| id.contains("::human-")), "l'oversight humain est cité comme évidence de l'art. 14");
}

#[test]
fn tampered_export_is_refused() {
    // I-S6-SUR-PROUVÉ : on altère un octet du payload exporté → la dérivation REFUSE (jamais d'évidence).
    let jsonl = build_via_s2();
    let tampered = jsonl.replacen("PASS", "FAIL", 1);
    let recs = SealedDag::records_from_jsonl(&tampered).unwrap();
    let err = derive(&recs, &key(), &templates::eu_ai_act_2026_06(), Framework::EuAiAct, &CaseScope::FullTenant);
    assert!(matches!(err, Err(RegError::NotProven { .. })), "export altéré → NotProven : {err:?}");
}

#[test]
fn csddd_supplier_dossier_is_the_roi() {
    // Le ROI : un dossier de diligence fournisseur, par-cas, re-dérivable — chaque ligne tracée à un nœud.
    let jsonl = build_via_s2();
    let recs = SealedDag::records_from_jsonl(&jsonl).unwrap();
    let mapping = templates::csddd_lksg_2026_06();
    let dossier = derive(&recs, &key(), &mapping, Framework::Csddd, &CaseScope::Case("SUP-2024".into())).expect("dossier CSDDD");

    // Le dossier documente la diligence (art. 11 couvert) et identifie le risque (art. 8 via l'évidence).
    let doc = dossier.findings.iter().find(|f| f.citation == "art. 11").unwrap();
    assert!(!doc.evidence.is_empty(), "la documentation de diligence cite les nœuds du dossier");
    let risk = dossier.findings.iter().find(|f| f.citation == "art. 8").unwrap();
    assert!(risk.evidence.iter().any(|id| id == "SUP-2024::ev-pol"), "le risque est identifié via l'évidence figée");
    // Honnêteté : un trou CSDDD est déclaré, et l'artefact n'affirme pas la conformité du fournisseur.
    assert!(dossier.coverage_summary.out_of_dag_scope >= 1, "un trou CSDDD déclaré");
    let md = render_md(&dossier);
    assert!(md.to_lowercase().contains("outillé ≠ conforme"));
    assert!(!md.to_lowercase().contains("compliant"));

    // Le sous-arbre de bundle voyage avec le mapping (re-dérivable par un tiers).
    let files = reg_subtree(&dossier, &mapping);
    assert!(files.iter().any(|(n, _)| n == "reg/csddd.md"));
    assert!(files.iter().any(|(n, _)| n == "reg/mapping.2026-06.json"));
    assert!(files.iter().any(|(n, _)| n == "reg/REDERIVE.md"));
}

#[test]
fn lksg_dossier_derives_for_the_other_framework() {
    // Le même registre sert les DEUX cadres : derive(Lksg) produit le dossier allemand.
    let jsonl = build_via_s2();
    let recs = SealedDag::records_from_jsonl(&jsonl).unwrap();
    let mapping = templates::csddd_lksg_2026_06();
    let lksg = derive(&recs, &key(), &mapping, Framework::Lksg, &CaseScope::Case("SUP-2024".into())).expect("dossier LkSG");
    assert_eq!(lksg.framework, Framework::Lksg);
    // §10(1) documentation couverte ; revue (oversight) cite le nœud humain.
    assert!(lksg.findings.iter().any(|f| f.citation == "§10(1)" && !f.evidence.is_empty()));
    let revue = lksg.findings.iter().find(|f| f.citation == "revue").unwrap();
    assert!(revue.evidence.iter().any(|id| id.contains("::human-")), "la revue cite l'oversight scellé");
    assert!(lksg.coverage_summary.out_of_dag_scope >= 1, "un trou LkSG déclaré (§8/§9)");
}
