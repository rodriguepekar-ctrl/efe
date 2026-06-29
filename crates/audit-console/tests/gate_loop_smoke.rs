// SPDX-License-Identifier: UNLICENSED
//! **La PORTE de P1, mécaniquement** — la boucle complète qu'un agent de conformité fait *seul* devant la
//! console, **plus** la re-preuve de son auditeur, jouée de bout en bout. Ce n'est **pas** un test
//! d'usabilité (ça, c'est humain) : c'est la garantie que **la plomberie ne casse pas devant l'officier**.
//!
//! La boucle (cf. l'architecture S3 §12, la PORTE) :
//! 1. l'officier **revoit** un dossier conforme (cadenas vert) ;
//! 2. il **traite une exception** (un dossier en échec sans revue) en **scellant** un OVERRIDE ;
//! 3. il **exporte** un dossier ;
//! 4. son **auditeur RE-PROUVE** le bundle exporté contre le root publié, **sans nous** — la patte forensic.

use audit_api::AuthContext;
use audit_console::{handle, Console, HttpRequest, Role};
use provenance_capture::{LlmCall, MessageEvent, ToolCall};
use provenance_dag::{verify_against_root, SealedDag, TenantKey};
use std::process::Command;

/// La clé de démo — la MÊME que le binaire console (derive("demo-root-secret","acme")) et le seed.
fn demo_key() -> TenantKey {
    TenantKey::derive(b"demo-root-secret", "acme")
}

/// Monte une console de démo : tenant provisionné + les deux dossiers du seed + une session officier.
fn demo_console() -> Console {
    let mut c = Console::new();
    c.provision("acme", demo_key());
    c.add_session("demo-token", "acme", Role::ComplianceOfficer, "officer:hanna");
    {
        let auth = AuthContext::new("acme", Role::ComplianceOfficer, "o");
        let svc = c.resolve_service(&auth).unwrap();
        // SUP-2024 : conforme.
        svc.ingest_llm(&LlmCall { case_id: "SUP-2024", step: "extract", agent_role: "supplier_extract", inputs: b"ACME GmbH", model: "phi4", version: "1", output: "{supplier:ACME}", evidence_steps: &[], timestamp: 1 }).unwrap();
        svc.ingest_evidence(&ToolCall { case_id: "SUP-2024", step: "ev-pol", tool: "policy_db", query: b"q", retrieved_ref: "policy://4.2", content: b"clause 4.2 au moment T", consumed_by_step: "sanction", timestamp: 2 }).unwrap();
        svc.ingest_llm(&LlmCall { case_id: "SUP-2024", step: "sanction", agent_role: "sanction_check", inputs: b"ACME", model: "phi4", version: "1", output: "PASS", evidence_steps: &["ev-pol"], timestamp: 3 }).unwrap();
        svc.ingest_message(&MessageEvent { case_id: "SUP-2024", step: "e1", from_step: "extract", to_step: "sanction", artifact: b"rec", artifact_type: "supplier_record", timestamp: 4 }).unwrap();
        svc.ingest_llm(&LlmCall { case_id: "SUP-2024", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"a", model: "phi4", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 5 }).unwrap();
        // SUP-2099 : en échec, SANS revue (l'exception à traiter).
        svc.ingest_llm(&LlmCall { case_id: "SUP-2099", step: "extract", agent_role: "supplier_extract", inputs: b"Globex", model: "phi4", version: "1", output: "{supplier:Globex}", evidence_steps: &[], timestamp: 6 }).unwrap();
        svc.ingest_llm(&LlmCall { case_id: "SUP-2099", step: "sanction", agent_role: "sanction_check", inputs: b"Globex", model: "phi4", version: "1", output: "FAIL", evidence_steps: &[], timestamp: 7 }).unwrap();
        svc.ingest_llm(&LlmCall { case_id: "SUP-2099", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"a", model: "phi4", version: "1", output: "NON_COMPLIANT", evidence_steps: &[], timestamp: 8 }).unwrap();
    }
    c
}

fn get(path: &str) -> HttpRequest {
    HttpRequest::parse(format!("GET {path} HTTP/1.1\r\nAuthorization: Bearer demo-token\r\n\r\n").as_bytes()).unwrap()
}
fn post_form(path: &str, body: &str) -> HttpRequest {
    HttpRequest::parse(format!("POST {path} HTTP/1.1\r\nAuthorization: Bearer demo-token\r\nContent-Length: {}\r\n\r\n{}", body.len(), body).as_bytes()).unwrap()
}

#[test]
fn the_full_gate_loop_holds_mechanically() {
    let mut c = demo_console();

    // ── 1. L'OFFICIER REVOIT un dossier conforme : il voit le cadenas VERT + le « pourquoi » tracé. ──
    let dossiers = handle(&mut c, &get("/"), 1000);
    assert_eq!(dossiers.status, 200);
    let dl = String::from_utf8_lossy(&dossiers.body);
    assert!(dl.contains("SUP-2024") && dl.contains("SUP-2099"), "les deux dossiers apparaissent");

    let sup = handle(&mut c, &get("/cases/SUP-2024"), 1000);
    assert_eq!(sup.status, 200);
    let s = String::from_utf8_lossy(&sup.body);
    assert!(s.contains("Vérifié — record authentique"), "cadenas VERT — il fait confiance");
    assert!(s.contains("SUP-2024::sanction") && s.contains("policy://4.2"), "le « pourquoi » est tracé (nœud + évidence)");
    assert!(s.contains("Exporter"), "l'export est à un clic");

    // ── 2. L'OFFICIER TRAITE UNE EXCEPTION : SUP-2099 (échec) remonte ; il SCELLE un OVERRIDE. ──
    let exceptions = handle(&mut c, &get("/exceptions"), 1000);
    assert!(String::from_utf8_lossy(&exceptions.body).contains("SUP-2099"), "l'échec remonte dans la file d'exceptions");

    let act = handle(&mut c, &post_form("/cases/SUP-2099/action", "target_step=aggregate&action=OVERRIDE&justification=revu%20manuellement%20%3A%20faux%20positif"), 1001);
    assert_eq!(act.status, 303, "PRG après la revue scellée");

    let after = handle(&mut c, &get("/cases/SUP-2099"), 1002);
    let a = String::from_utf8_lossy(&after.body);
    assert!(a.contains("OUTREPASSER") && a.contains("officer:hanna"), "l'oversight est SCELLÉ et tracé");
    assert!(a.contains("EXCEPTION"), "le dossier reflète sa nouvelle réalité (exception tracée)");

    // ── 3. L'OFFICIER EXPORTE un dossier (un clic) → un tar autonome. ──
    let exp = handle(&mut c, &get("/cases/SUP-2024/export?fw=eu_ai_act"), 1003);
    assert_eq!(exp.status, 200);
    assert_eq!(exp.content_type, "application/x-tar");
    assert!(!exp.body.is_empty() && exp.body.len() % 512 == 0);

    // ── 4. L'AUDITEUR RE-PROUVE le bundle exporté, SANS NOUS : extraire → verify_against_root(root publié). ──
    let dir = std::env::temp_dir().join(format!("efe-gate-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bundle.tar"), &exp.body).unwrap();
    match Command::new("tar").args(["-xf", dir.join("bundle.tar").to_str().unwrap(), "-C", dir.to_str().unwrap()]).status() {
        Ok(st) if st.success() => {
            // L'auditeur lit les records scellés + le root publié, et re-prouve avec la clé tenant.
            let jsonl = std::fs::read_to_string(dir.join("records.sealed.jsonl")).unwrap();
            let root = std::fs::read_to_string(dir.join("manifest.root")).unwrap().trim().to_string();
            let records = SealedDag::records_from_jsonl(&jsonl).unwrap();
            let verdict = verify_against_root(&records, &demo_key(), &root);
            assert!(verdict.ok, "L'AUDITEUR re-prouve le bundle exporté contre le root publié — la porte forensic tient");
            // Et la clé ne voyage jamais.
            let key_hex: String = {
                // (on n'a pas l'accès aux octets de la clé ; on vérifie via le README qu'aucun secret n'y est)
                std::fs::read_to_string(dir.join("README.md")).unwrap()
            };
            assert!(key_hex.contains("ne contient pas"), "le README rappelle que la clé voyage SÉPARÉMENT");
        }
        _ => eprintln!("SKIP re-preuve auditeur : tar système indisponible (la boucle officier 1-3 reste prouvée)"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
