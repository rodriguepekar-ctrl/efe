// SPDX-License-Identifier: UNLICENSED
//! **I-S6-VUE-REDÉRIVABLE (hors-process, depuis le VRAI bundle S4) — la composition S6 × S4 (archi §9).**
//!
//! Le test que la suite précédente n'avait pas : non pas re-dériver en mémoire depuis la chaîne d'export
//! S2, mais **assembler un vrai bundle forensic** (records.sealed.jsonl + vérifieur vendoré + MANIFEST +
//! README), y **embarquer** le sous-arbre `reg/` (l'artefact rendu + la lecture versionnée + le mode
//! d'emploi), puis — **comme un tiers** — re-lire `records.sealed.jsonl` **sur le disque**, **re-prouver
//! contre le root publié** (la patte « verify → PASS » de l'équation §9), et **re-dériver** l'artefact
//! réglementaire pour exiger l'égalité **octet-à-octet** avec le `reg/<cadre>.md` du bundle.
//!
//! C'est la matérialisation complète de « l'artefact voyage dans le bundle de S4, et il y est re-dérivable,
//! zéro confiance neuve » : la lecture (mapping) voyage, donc le tiers re-dérive **sans nous**.

use audit_api::{AuditService, AuthContext, HumanActionInput, ProjectionConfig, Role};
use forensic_export::{assemble, BundleInput, BundleOpts};
use provenance_capture::{LlmCall, MessageEvent, ToolCall};
use provenance_dag::{verify_against_root, HumanAction, SealedDag, TenantKey};
use reg_evidence::{derive, reg_subtree, render_md, templates, CaseScope, Framework, MappingSet};

const KEY_BYTES: [u8; 32] = [22u8; 32];
fn key() -> TenantKey {
    TenantKey::from_bytes(KEY_BYTES)
}
fn officer() -> AuthContext {
    AuthContext::new("acme", Role::ComplianceOfficer, "officer:hanna")
}

/// Un cas « conformité fournisseur » complet via S1 (capture) + le gate humain S2 ; rend l'`ExportContent`.
fn s2_export() -> audit_api::ExportContent {
    let mut svc = AuditService::new(key(), ProjectionConfig::default());
    svc.ingest_llm(&LlmCall { case_id: "SUP-2024", step: "extract", agent_role: "supplier_extract", inputs: b"dossier ACME", model: "phi4", version: "1", output: "{supplier:SUP-2024}", evidence_steps: &[], timestamp: 1 }).unwrap();
    svc.ingest_evidence(&ToolCall { case_id: "SUP-2024", step: "ev-pol", tool: "policy_db", query: b"sanctions?", retrieved_ref: "policy://4.2", content: b"clause 4.2 au moment T", consumed_by_step: "sanction", timestamp: 2 }).unwrap();
    svc.ingest_llm(&LlmCall { case_id: "SUP-2024", step: "sanction", agent_role: "sanction_check", inputs: b"nom: ACME", model: "phi4", version: "1", output: "PASS", evidence_steps: &["ev-pol"], timestamp: 3 }).unwrap();
    svc.ingest_message(&MessageEvent { case_id: "SUP-2024", step: "e1", from_step: "extract", to_step: "sanction", artifact: b"supplier_record", artifact_type: "supplier_record", timestamp: 4 }).unwrap();
    svc.ingest_llm(&LlmCall { case_id: "SUP-2024", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"agg", model: "phi4", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 5 }).unwrap();
    svc.record_human_action("SUP-2024", &HumanActionInput { target_step: "aggregate".into(), action: HumanAction::Approve, justification: "revue conforme".into() }, &officer(), 6).unwrap();
    svc.export(None, &officer(), 100).expect("export S2")
}

#[test]
fn third_party_rederives_the_reg_artifact_from_the_real_bundle() {
    let exp = s2_export();
    let mapping = templates::eu_ai_act_2026_06();
    let scope = CaseScope::Case("SUP-2024".into());

    // KOVANT : re-prove + derive (in-process), then ASSEMBLE a real forensic bundle embedding reg/.
    let records = SealedDag::records_from_jsonl(&exp.records_jsonl).unwrap();
    let artifact = derive(&records, &key(), &mapping, Framework::EuAiAct, &scope).expect("dérivation");
    let expected_md = render_md(&artifact);

    let bundle_dir = std::env::temp_dir().join(format!("s6xs4-bundle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&bundle_dir);
    let input = BundleInput {
        records_jsonl: exp.records_jsonl.clone(),
        manifest_root: exp.manifest_root.clone(),
        report: exp.report.clone(),
        sealed_scope: exp.sealed_scope.to_string(),
        report_cases: exp.report_cases.clone(),
    };
    let opts = BundleOpts {
        tenant_label: "acme".into(),
        reg_attachments: reg_subtree(&artifact, &mapping), // l'artefact + la lecture + le mode d'emploi voyagent
        ..Default::default()
    };
    let m = assemble(&input, &key(), &opts, &bundle_dir).expect("assemblage du bundle");

    // Les fichiers reg/ sont DANS le bundle, couverts par le MANIFEST, et le README gagne sa section.
    for f in ["records.sealed.jsonl", "manifest.root", "reg/eu_ai_act.md", "reg/mapping.2026-06.json", "reg/REDERIVE.md"] {
        assert!(bundle_dir.join(f).exists(), "fichier de bundle manquant : {f}");
    }
    let manifest_listed: Vec<&str> = m.files.iter().map(|(p, _)| p.as_str()).collect();
    for f in ["reg/eu_ai_act.md", "reg/mapping.2026-06.json", "reg/REDERIVE.md"] {
        assert!(manifest_listed.contains(&f), "le MANIFEST couvre {f}");
    }
    assert!(std::fs::read_to_string(bundle_dir.join("README.md")).unwrap().contains("## Évidence réglementaire"), "le README du bundle gagne la section reg");

    // ── LE TIERS, hors-process : il ne détient que le bundle sur disque + sa clé tenant. ──
    // 1. Re-lit records.sealed.jsonl DEPUIS LE DISQUE.
    let disk_jsonl = std::fs::read_to_string(bundle_dir.join("records.sealed.jsonl")).unwrap();
    let disk_records = SealedDag::records_from_jsonl(&disk_jsonl).unwrap();
    // 2. VERIFY → PASS contre le root publié (la patte « verify d'abord » de l'équation §9).
    let published_root = std::fs::read_to_string(bundle_dir.join("manifest.root")).unwrap().trim().to_string();
    assert!(verify_against_root(&disk_records, &key(), &published_root).ok, "le tiers re-prouve contre le root publié");
    // 3. Lit la LECTURE versionnée embarquée (le mapping voyage → re-dérivable sans nous).
    let disk_mapping: MappingSet = serde_json::from_str(&std::fs::read_to_string(bundle_dir.join("reg/mapping.2026-06.json")).unwrap()).unwrap();
    assert_eq!(disk_mapping.set_hash(), mapping.set_hash(), "le mapping embarqué reproduit le hash (re-dérivable)");
    // 4. RE-DÉRIVE l'artefact, et exige l'égalité OCTET-À-OCTET avec reg/eu_ai_act.md du bundle.
    let rederived = derive(&disk_records, &key(), &disk_mapping, Framework::EuAiAct, &scope).expect("re-dérivation tiers");
    let rederived_md = render_md(&rederived);
    let on_disk_md = std::fs::read_to_string(bundle_dir.join("reg/eu_ai_act.md")).unwrap();
    assert_eq!(rederived_md, on_disk_md, "le tiers re-dérive le reg/<cadre>.md octet-à-octet (zéro confiance neuve)");
    assert_eq!(rederived_md, expected_md, "et c'est bien l'artefact que Kovant a produit");
    // L'artefact est lié à l'état du DAG publié.
    assert_eq!(rederived.manifest_root, published_root, "l'artefact est lié au root publié");

    let _ = std::fs::remove_dir_all(&bundle_dir);
}

#[test]
fn bundle_without_reg_has_no_reg_subtree() {
    // Contrôle négatif : un bundle assemblé SANS pièces reg/ n'a ni dossier reg/ ni section README.
    let exp = s2_export();
    let bundle_dir = std::env::temp_dir().join(format!("s4-noreg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&bundle_dir);
    let input = BundleInput {
        records_jsonl: exp.records_jsonl.clone(),
        manifest_root: exp.manifest_root.clone(),
        report: exp.report.clone(),
        sealed_scope: exp.sealed_scope.to_string(),
        report_cases: exp.report_cases.clone(),
    };
    assemble(&input, &key(), &BundleOpts { tenant_label: "acme".into(), ..Default::default() }, &bundle_dir).unwrap();
    assert!(!bundle_dir.join("reg").exists(), "aucun dossier reg/ sans pièce");
    assert!(!std::fs::read_to_string(bundle_dir.join("README.md")).unwrap().contains("Évidence réglementaire"));
    let _ = std::fs::remove_dir_all(&bundle_dir);
}
