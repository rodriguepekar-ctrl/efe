// SPDX-License-Identifier: UNLICENSED
//! **I-S2-AIRGAP** (§7/§12) — test d'INTÉGRATION : l'audit-api **sert, re-prouve et exporte** entièrement
//! **hors-ligne** (aucune dépendance réseau dans le crate — air-gap par construction), et la **garde de
//! souveraineté** ré-exportée (§7) refuse tout endpoint non-local. Ce test prend possession de l'invariant
//! d'air-gap pour qu'il cesse d'être le seul non-testé.
//!
//! Note de portée : S2 n'a **pas de socket sortant** (le routeur est une fonction pure ; la coquille de
//! transport, déférée, appliquerait `assert_local` à son bind). On vérifie donc (a) que le cycle de vie
//! complet tourne sans aucune E/S réseau, et (b) que le contrôle d'egress hérité est bien exposé et correct.

use audit_api::{assert_local, is_local_endpoint, route, AuthContext, Method, ProjectionConfig, Request, Role, TenantRegistry};
use provenance_capture::LlmCall;
use provenance_dag::TenantKey;

fn req(method: Method, path: &str, auth: AuthContext) -> Request {
    Request { method, path: path.into(), query: vec![], auth, body: None, now: 100 }
}

#[test]
fn full_lifecycle_runs_offline() {
    let mut reg = TenantRegistry::new();
    reg.provision("acme", TenantKey::derive(b"root", "acme"), ProjectionConfig::default());
    let auth = AuthContext::new("acme", Role::ComplianceOfficer, "officer:hanna");
    {
        let svc = reg.resolve(&auth).unwrap();
        svc.ingest_llm(&LlmCall { case_id: "SUP-A", step: "sanction", agent_role: "sanction_check", inputs: b"acme", model: "phi4", version: "1", output: "PASS", evidence_steps: &[], timestamp: 1 }).unwrap();
        svc.ingest_llm(&LlmCall { case_id: "SUP-A", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"agg", model: "phi4", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 2 }).unwrap();
    }
    // Servir une décision, la rendre, re-prouver (froid), exporter — TOUT en mémoire, zéro réseau.
    assert_eq!(route(&mut reg, &req(Method::Get, "/tenants/acme/cases/SUP-A", auth.clone())).status, 200);
    assert_eq!(route(&mut reg, &req(Method::Get, "/tenants/acme/cases/SUP-A/report", auth.clone())).status, 200);
    assert_eq!(route(&mut reg, &req(Method::Post, "/tenants/acme/reprove", auth.clone())).status, 200);
    let exp = route(&mut reg, &req(Method::Get, "/tenants/acme/export", auth));
    assert_eq!(exp.status, 200);
    assert!(exp.body.contains("manifest_root"), "export produit un bundle re-provable, hors-ligne");
}

#[test]
fn sovereignty_guard_is_exposed_and_denies_egress() {
    // §7 : la garde héritée est ré-exportée par S2 et reste correcte (egress refusé par défaut).
    assert!(is_local_endpoint("http://127.0.0.1:11434"));
    assert!(is_local_endpoint("http://localhost:8000"));
    assert!(is_local_endpoint("http://[::1]:8080"));
    assert!(assert_local("http://localhost:8000").is_ok());
    for external in ["https://api.openai.com/v1", "http://evil.com", "http://127.evil.com", "http://evil.com#@127.0.0.1"] {
        assert!(!is_local_endpoint(external), "endpoint externe refusé : {external}");
        assert!(assert_local(external).is_err());
    }
}
