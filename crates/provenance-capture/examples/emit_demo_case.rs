// SPDX-License-Identifier: UNLICENSED
//! Émet un **cas de conformité fournisseur** réel, scellé, sur disque — le « bundle » qu'un client
//! remet à un auditeur, accompagné du binaire `provenance-verify` et de la clé tenant (en hex).
//!
//! Usage : `cargo run -p provenance-capture --example emit_demo_case -- <dossier_sortie>`
//! Écrit `<dossier>/records.jsonl` et imprime sur stdout la clé hex + le manifest-root attendu.
//! Le tiers fait ensuite, **sans rien d'autre que `provenance-verify`** :
//!   `provenance-verify <dossier>/records.jsonl --key-hex <hex> --root <root>`

use std::path::PathBuf;

use provenance_capture::{CaptureStore, HumanReview, LlmCall, MessageEvent, ToolCall};
use provenance_dag::{HumanAction, TenantKey};

fn main() {
    let out_dir = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    // Octets de clé FIXES pour la démo (en prod : dérivée par tenant, jamais en clair).
    let key_bytes = [0x2au8; 32];
    let key = TenantKey::from_bytes(key_bytes);
    let mut store = CaptureStore::new(key);
    let case = "SUP-2024-0917";

    // Un cas complet, dans l'ordre causal — exactement ce que captureraient les taps en runtime.
    store.record_llm(&LlmCall { case_id: case, step: "extract", agent_role: "supplier_extract", inputs: b"facture.pdf", model: "phi4-mini", version: "1", output: "{supplier: ACME GmbH, pays: DE}", evidence_steps: &[], timestamp: 1 }).unwrap();
    store.record_evidence(&ToolCall { case_id: case, step: "ev-sanctions", tool: "eu_sanctions_db", query: b"ACME GmbH", retrieved_ref: "eu-sanctions://2024-09", content: b"no-match", consumed_by_step: "sanction", timestamp: 2 }).unwrap();
    store.record_llm(&LlmCall { case_id: case, step: "sanction", agent_role: "sanction_check", inputs: b"ACME GmbH; DE", model: "phi4-mini", version: "1", output: "PASS", evidence_steps: &["ev-sanctions"], timestamp: 3 }).unwrap();
    store.record_message(&MessageEvent { case_id: case, step: "msg-1", from_step: "extract", to_step: "sanction", artifact: b"supplier_record", artifact_type: "supplier_record", timestamp: 4 }).unwrap();
    store.record_llm(&LlmCall { case_id: case, step: "aggregate", agent_role: "verdict_aggregate", inputs: b"sanction=PASS", model: "phi4-mini", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 5 }).unwrap();
    store.record_human(&HumanReview { case_id: case, step: "review-1", human_id: "officer:hanna.svensson", action: HumanAction::Approve, target_step: "aggregate", justification: "revue conforme — sanctions vérifiées", timestamp: 6 }).unwrap();

    let path = out_dir.join("records.jsonl");
    store.save_local(&path).expect("écriture locale du bundle");

    let key_hex: String = key_bytes.iter().map(|b| format!("{b:02x}")).collect();
    println!("records : {}", path.display());
    println!("clé hex : {key_hex}");
    println!("root    : {}", store.manifest_root());
    println!("re-preuve interne : {}", if store.reprove().ok { "OK" } else { "ÉCHEC" });
}
