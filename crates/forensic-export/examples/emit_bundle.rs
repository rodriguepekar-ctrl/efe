// SPDX-License-Identifier: UNLICENSED
//! Émet un **bundle forensic** réel sur disque — pour le test end-to-end « trustless » : un tiers
//! re-compile la source vendorée du vérifieur et re-prouve le bundle, hors-ligne, sans rien d'EFE/Kovant.
//!
//! Usage : `cargo run -p forensic-export --example emit_bundle -- <dossier_sortie>`
//! Imprime le chemin du bundle, la clé hex (à donner au vérifieur — elle n'est PAS dans le bundle) et le root.

use forensic_export::{assemble, BundleInput, BundleOpts};
use provenance_dag::{Node, SealedDag, TenantKey};

fn main() {
    let out = std::env::args().nth(1).expect("usage : emit_bundle <dossier_sortie>");
    let key_bytes = [0x2au8; 32];
    let k = TenantKey::from_bytes(key_bytes);

    // Un cas de conformité fournisseur réaliste, scellé.
    let case = "SUP-2024-0917";
    let mut d = SealedDag::new();
    d.append(&k, Node::AgentAct { node_id: format!("{case}::extract"), agent_role: "supplier_extract".into(), inputs_hash: "h1".into(), model: "phi4-mini".into(), version: "1".into(), output: "{supplier: ACME GmbH}".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
    d.append(&k, Node::Evidence { evidence_id: format!("{case}::ev-sanctions"), tool: "eu_sanctions_db".into(), query_hash: "q".into(), retrieved_ref: "eu-sanctions://2024-09".into(), content_hash: "c".into(), consumed_by_node: format!("{case}::sanction"), timestamp: 2 }).unwrap();
    d.append(&k, Node::AgentAct { node_id: format!("{case}::sanction"), agent_role: "sanction_check".into(), inputs_hash: "h2".into(), model: "phi4-mini".into(), version: "1".into(), output: "PASS".into(), evidence_refs: vec![format!("{case}::ev-sanctions")], timestamp: 3 }).unwrap();
    d.append(&k, Node::AgentAct { node_id: format!("{case}::aggregate"), agent_role: "verdict_aggregate".into(), inputs_hash: "h3".into(), model: "phi4-mini".into(), version: "1".into(), output: "COMPLIANT".into(), evidence_refs: vec![], timestamp: 4 }).unwrap();

    let input = BundleInput {
        records_jsonl: d.to_jsonl(),
        manifest_root: d.manifest_root(),
        report: format!("# Dossier {case} — COMPLIANT\n\n**Verdict** : COMPLIANT  ·  _(nœud scellé `{case}::aggregate`)_\n\n## Contrôles\n- sanction_check : PASS  ·  _[`{case}::sanction`]_\n"),
        sealed_scope: "full-tenant-dag (re-provable depuis la genèse)".into(),
        report_cases: vec![case.to_string()],
    };
    let opts = BundleOpts { tenant_label: "acme-corp".into(), ..Default::default() };
    let m = assemble(&input, &k, &opts, std::path::Path::new(&out)).expect("assemblage du bundle");

    // Clé de DÉMONSTRATION (fixe), imprimée ici uniquement pour le test trustless end-to-end. En
    // production, K_tenant ne doit JAMAIS être imprimée/journalisée : elle voyage hors-bande (cf. le
    // README du bundle, « La clé voyage SÉPARÉMENT »), et n'entre jamais dans le bundle.
    let key_hex: String = key_bytes.iter().map(|b| format!("{b:02x}")).collect();
    println!("bundle  : {}", m.dir.display());
    println!("clé hex : {key_hex}");
    println!("root    : {}", m.manifest_root);
    println!("fichiers: {}", m.files.len());
}
