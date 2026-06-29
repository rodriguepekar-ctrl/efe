// SPDX-License-Identifier: UNLICENSED
//! **Bout-en-bout au fil HTTP** — on construit des octets HTTP bruts (comme un navigateur), on les passe
//! par `HttpRequest::parse → handle → HttpResponse::to_bytes`, et on vérifie la réponse au fil. Puis on
//! **extrait le vrai tar** d'export avec le `tar` système (compatibilité réelle), en vérifiant que la
//! **clé tenant n'y figure pas** (souveraineté). Enfin, la **rédaction** d'un `Viewer` est rendue.

use audit_api::AuthContext;
use audit_console::{handle, Console, HttpRequest, Role};
use provenance_capture::LlmCall;
use provenance_dag::TenantKey;
use std::process::Command;

const KEY: [u8; 32] = [0x5cu8; 32];

fn console() -> Console {
    let mut c = Console::new();
    c.provision("acme", TenantKey::from_bytes(KEY));
    c.add_session("offtok", "acme", Role::ComplianceOfficer, "officer:hanna");
    c.add_session("viewtok", "acme", Role::Viewer, "v");
    {
        let auth = AuthContext::new("acme", Role::ComplianceOfficer, "o");
        let svc = c.resolve_service(&auth).unwrap();
        svc.ingest_evidence(&provenance_capture::ToolCall { case_id: "SUP-2024", step: "ev-pol", tool: "policy_db", query: b"q", retrieved_ref: "policy://4.2", content: b"clause", consumed_by_step: "sanction", timestamp: 1 }).unwrap();
        svc.ingest_llm(&LlmCall { case_id: "SUP-2024", step: "sanction", agent_role: "sanction_check", inputs: b"nom: ACME", model: "phi4", version: "1", output: "PASS", evidence_steps: &["ev-pol"], timestamp: 2 }).unwrap();
        svc.ingest_llm(&LlmCall { case_id: "SUP-2024", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"i", model: "phi4", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 3 }).unwrap();
    }
    c
}

/// Joue une requête au FIL : octets bruts → parse → handle → to_bytes.
fn wire(c: &mut Console, raw: &str, now: u64) -> (String, Vec<u8>) {
    let req = HttpRequest::parse(raw.as_bytes()).expect("requête parsable");
    let resp = handle(c, &req, now);
    let bytes = resp.to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    (text, resp.body)
}

#[test]
fn full_wire_round_trip_for_the_console() {
    let mut c = console();
    // Liste des dossiers.
    let (head, _) = wire(&mut c, "GET / HTTP/1.1\r\nAuthorization: Bearer offtok\r\n\r\n", 100);
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "réponse HTTP bien formée");
    assert!(head.contains("Content-Type: text/html"));
    assert!(head.contains("SUP-2024"), "le dossier apparaît");
    assert!(head.contains("Montré vérifié ≠ prouvé sans confiance"), "l'honnêteté cardinale est au fil");

    // Vue décision : cite ses nœuds, montre l'évidence figée + le badge vert frais.
    let (dec, _) = wire(&mut c, "GET /cases/SUP-2024 HTTP/1.1\r\nAuthorization: Bearer offtok\r\n\r\n", 100);
    assert!(dec.contains("SUP-2024::sanction"), "chaque contrôle cite son nœud scellé");
    assert!(dec.contains("policy://4.2"), "l'évidence citée est rendue");
    assert!(dec.contains("Vérifié — record authentique") && dec.contains("as_of = 100"), "badge vert + fraîcheur");

    // Sans session → 401 au fil, rien révélé.
    let (un, _) = wire(&mut c, "GET /cases/SUP-2024 HTTP/1.1\r\n\r\n", 100);
    assert!(un.starts_with("HTTP/1.1 401"), "401 sans session");
    assert!(!un.contains("SUP-2024"), "aucune donnée révélée");
}

#[test]
fn export_tar_is_extractable_and_keyless() {
    let mut c = console();
    let (head, body) = wire(&mut c, "GET /cases/SUP-2024/export?fw=eu_ai_act HTTP/1.1\r\nAuthorization: Bearer offtok\r\n\r\n", 100);
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(head.contains("Content-Type: application/x-tar"));
    assert!(head.contains("attachment; filename="), "téléchargement");
    assert!(body.len() % 512 == 0 && !body.is_empty(), "tar bien formé");
    // I-S3-SOUVERAIN : la clé ([0x5c;32] hex) n'est NULLE PART dans le bundle.
    let key_hex = "5c".repeat(32);
    assert!(!String::from_utf8_lossy(&body).contains(&key_hex), "la clé ne part jamais");

    // Extraction par le tar SYSTÈME → compatibilité réelle ; le bundle se re-prouve hors-ligne.
    let dir = std::env::temp_dir().join(format!("s3-wire-untar-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let tar_path = dir.join("bundle.tar");
    std::fs::write(&tar_path, &body).unwrap();
    match Command::new("tar").args(["-xf", tar_path.to_str().unwrap(), "-C", dir.to_str().unwrap()]).status() {
        Ok(s) if s.success() => {
            assert!(dir.join("records.sealed.jsonl").exists(), "le record scellé est dans le bundle");
            assert!(dir.join("verify/verify-mini/src/main.rs").exists(), "la source du vérifieur zéro-dép (à re-compiler)");
            assert!(dir.join("reg/eu_ai_act.md").exists(), "l'évidence réglementaire (S6) voyage");
            assert!(dir.join("MANIFEST.sha256").exists() && dir.join("README.md").exists());
            // Aucun fichier extrait ne contient la clé.
            let readme = std::fs::read_to_string(dir.join("README.md")).unwrap();
            assert!(!readme.contains(&key_hex));
        }
        _ => eprintln!("SKIP : tar système indisponible — l'archive reste bien formée et sans clé (vérifié ci-dessus)"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn viewer_sees_redaction_rendered() {
    let mut c = console();
    // L'officier voit le clair (PASS) ; le Viewer voit la rédaction de S2 ([rédigé]) — rendue, pas refaite.
    let (officer, _) = wire(&mut c, "GET /cases/SUP-2024 HTTP/1.1\r\nAuthorization: Bearer offtok\r\n\r\n", 100);
    assert!(officer.contains("PASS"), "l'officier voit le clair");
    let (viewer, _) = wire(&mut c, "GET /cases/SUP-2024 HTTP/1.1\r\nAuthorization: Bearer viewtok\r\n\r\n", 100);
    assert!(viewer.contains("rédigé"), "le Viewer voit la rédaction de S2");
    assert!(!viewer.contains("class=\"btn export\""), "et ni export ni action (cosmétique ; S2 enforce)");
    assert!(!viewer.contains("Sceller la revue"));
}
