// SPDX-License-Identifier: UNLICENSED
//! **Seed de démo pour la validation de la porte P1.** Génère un `records.sealed.jsonl` réaliste — scellé
//! avec la **clé de démo de la console** (`derive("demo-root-secret", "acme")`) — que le binaire
//! `audit-console` réhydrate pour la démo devant un agent de conformité.
//!
//! Deux dossiers, exprès :
//! - **SUP-2024** — un cas **conforme** complet (extraction → évidence figée → contrôle sanctions PASS →
//!   arête → agrégation COMPLIANT). L'officier le **revoit**, **fait confiance** au cadenas vert, l'**exporte**.
//! - **SUP-2099** — un cas **en échec** (criblage FAIL) **sans revue humaine** : l'officier doit **traiter
//!   l'exception** dans la console (sceller un OVERRIDE/ESCALATE) — c'est la patte « gérer une exception ».
//!
//! Lancement :
//!   cargo run -p audit-console --example seed_demo -- /tmp/demo_records.sealed.jsonl
//! puis la console :
//!   EFE_CONSOLE_RECORDS=/tmp/demo_records.sealed.jsonl cargo run -p audit-console

use provenance_capture::{CaptureStore, LlmCall, MessageEvent, ToolCall};
use provenance_dag::TenantKey;

fn main() {
    // DOIT correspondre aux défauts du binaire console (EFE_CONSOLE_ROOT / EFE_CONSOLE_TENANT) — sinon la
    // re-preuve échouerait à la réhydratation (la clé est dérivée du même secret racine + tenant).
    let key = TenantKey::derive(b"demo-root-secret", "acme");
    let mut store = CaptureStore::new(key);

    // ── SUP-2024 : un dossier fournisseur CONFORME (la boucle « revoir → faire confiance → exporter ») ──
    store.record_llm(&LlmCall { case_id: "SUP-2024", step: "extract", agent_role: "supplier_extract", inputs: b"dossier fournisseur ACME GmbH", model: "phi4-mini", version: "1", output: "{supplier:\"ACME GmbH\",pays:\"DE\"}", evidence_steps: &[], timestamp: 1 }).unwrap();
    store.record_evidence(&ToolCall { case_id: "SUP-2024", step: "ev-pol", tool: "policy_db", query: b"sanctions UE/OFAC ?", retrieved_ref: "policy://sanctions/4.2", content: b"Clause 4.2 - criblage listes UE+OFAC, version au moment T", consumed_by_step: "sanction", timestamp: 2 }).unwrap();
    store.record_llm(&LlmCall { case_id: "SUP-2024", step: "sanction", agent_role: "sanction_check", inputs: b"ACME GmbH; DE", model: "phi4-mini", version: "1", output: "PASS", evidence_steps: &["ev-pol"], timestamp: 3 }).unwrap();
    store.record_message(&MessageEvent { case_id: "SUP-2024", step: "e1", from_step: "extract", to_step: "sanction", artifact: b"supplier_record", artifact_type: "supplier_record", timestamp: 4 }).unwrap();
    store.record_llm(&LlmCall { case_id: "SUP-2024", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"verdicts", model: "phi4-mini", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 5 }).unwrap();

    // ── SUP-2099 : un dossier EN ÉCHEC sans revue (la patte « traiter une exception » — l'officier scelle) ──
    store.record_llm(&LlmCall { case_id: "SUP-2099", step: "extract", agent_role: "supplier_extract", inputs: b"dossier fournisseur Globex SARL", model: "phi4-mini", version: "1", output: "{supplier:\"Globex SARL\",pays:\"FR\"}", evidence_steps: &[], timestamp: 6 }).unwrap();
    store.record_evidence(&ToolCall { case_id: "SUP-2099", step: "ev-esg", tool: "esg_registry", query: b"controverses ESG ?", retrieved_ref: "esg://controversy/2025-114", content: b"Controverse environnementale ouverte, gravite elevee, au moment T", consumed_by_step: "sanction", timestamp: 7 }).unwrap();
    store.record_llm(&LlmCall { case_id: "SUP-2099", step: "sanction", agent_role: "sanction_check", inputs: b"Globex SARL; FR", model: "phi4-mini", version: "1", output: "FAIL", evidence_steps: &["ev-esg"], timestamp: 8 }).unwrap();
    store.record_llm(&LlmCall { case_id: "SUP-2099", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"verdicts", model: "phi4-mini", version: "1", output: "NON_COMPLIANT", evidence_steps: &[], timestamp: 9 }).unwrap();

    let path = std::env::args().nth(1).unwrap_or_else(|| "demo_records.sealed.jsonl".to_string());
    let jsonl = store.to_jsonl();
    std::fs::write(&path, format!("{jsonl}\n")).expect("écriture du seed");

    let v = store.reprove();
    eprintln!("✔ seed écrit : {path}");
    eprintln!("  {} records · manifest-root {}", v.n_records, store.manifest_root());
    eprintln!("  re-prouvable : {}", v.ok);
    eprintln!();
    eprintln!("Lancer la console :");
    eprintln!("  EFE_CONSOLE_RECORDS={path} cargo run -p audit-console");
    eprintln!("Puis ouvrir http://127.0.0.1:8088 avec l'en-tête  Authorization: Bearer demo-token");
    eprintln!("  (ou un cookie  session=demo-token)");
    eprintln!();
    eprintln!("Démo : SUP-2024 = conforme (revoir → cadenas vert → exporter) ;");
    eprintln!("       SUP-2099 = échec sans revue (file d'exceptions → sceller un OVERRIDE/ESCALATE).");
}
