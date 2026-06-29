// SPDX-License-Identifier: UNLICENSED
//! **I-S4-RÉCIT=VUE** (test d'intégration, forme FORTE) — le récit du bundle est **exactement
//! reproductible** depuis les records scellés : on produit un `ExportContent` via S2, on assemble le
//! bundle, puis on **re-projette + re-rend** depuis les records du bundle (avec le rendeur S2) et on exige
//! l'**égalité au caractère près** avec `report.md`. Toute divergence dénoncerait un récit trafiqué. (Le
//! bundle seul garantit la traçabilité par node_id ; la régénération exacte exige le rendeur S2, obtenu
//! séparément — ce que le README énonce honnêtement.)

use audit_api::{project, render_export, AuditService, AuthContext, IntegrityEnvelope, ProjectionConfig, Role};
use forensic_export::{assemble, BundleInput, BundleOpts};
use provenance_capture::LlmCall;
use provenance_dag::{reprove_dag, Record, SealedDag, TenantKey};

#[test]
fn report_is_exactly_reproducible_from_the_sealed_records() {
    let kb = [9u8; 32];
    let key = TenantKey::from_bytes(kb);
    let now = 100u64;

    // 1. Construire un cas via S2 et l'exporter (le récit est rendu par S2 à l'instant `now`).
    let mut svc = AuditService::new(TenantKey::from_bytes(kb), ProjectionConfig::default());
    svc.ingest_llm(&LlmCall { case_id: "SUP-A", step: "sanction", agent_role: "sanction_check", inputs: b"i", model: "phi4", version: "1", output: "PASS", evidence_steps: &[], timestamp: 1 }).unwrap();
    svc.ingest_llm(&LlmCall { case_id: "SUP-A", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"i", model: "phi4", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 2 }).unwrap();
    let officer = AuthContext::new("acme", Role::ComplianceOfficer, "officer"); // sees_unredacted → récit en clair
    let exp = svc.export(None, &officer, now).expect("export S2");
    let report_from_export = exp.report.clone();

    // 2. Assembler le bundle (le récit y est écrit verbatim).
    let input = BundleInput {
        records_jsonl: exp.records_jsonl,
        manifest_root: exp.manifest_root,
        report: exp.report,
        sealed_scope: exp.sealed_scope.to_string(),
        report_cases: exp.report_cases.clone(),
    };
    let dir = std::env::temp_dir().join(format!("s4-recit-strong-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assemble(&input, &key, &BundleOpts { tenant_label: "acme".into(), ..Default::default() }, &dir).expect("assemblage");

    // 3. Côté TIERS : re-projeter + re-rendre depuis les records DU BUNDLE, et diff  au caractère près.
    let jsonl = std::fs::read_to_string(dir.join("records.sealed.jsonl")).unwrap();
    let records = SealedDag::records_from_jsonl(&jsonl).unwrap();
    let cfg = ProjectionConfig::default();
    let case_recs: Vec<&Record> = records.iter().filter(|r| r.node.id().starts_with("SUP-A::")).collect();
    let view = project(&case_recs, "SUP-A", &cfg);
    let env = IntegrityEnvelope::from_verdict(&reprove_dag(&records, &key), now);
    let re_rendered = render_export(&[view], &env);

    let report_md = std::fs::read_to_string(dir.join("report.md")).unwrap();
    assert_eq!(report_md, report_from_export, "le bundle écrit le récit verbatim (pas de mutation à l'assemblage)");
    assert_eq!(re_rendered, report_md, "RÉCIT = VUE : le récit est EXACTEMENT reproductible depuis les records scellés");
    assert!(report_md.contains("SUP-A::sanction"), "le récit cite bien les nœuds scellés");

    let _ = std::fs::remove_dir_all(&dir);
}
