// SPDX-License-Identifier: UNLICENSED
//! **Le routeur transport-agnostique.** Modélise la sémantique HTTP des routes de S2 (§11) comme une
//! **fonction pure** `route(registry, request) -> response` : pas de socket, pas d'async — donc
//! **air-gap-testable** et **adversarial-reviewable** intégralement, exactement comme S0/S1. Une coquille
//! mince (hyper/axum) qui *authentifie* puis appelle `route` est tout ce qui manque pour servir en réseau ;
//! elle n'ajoute aucune logique de confiance (toute la confiance est ici, en Rust).
//!
//! Le routeur enforce l'**isolation tenant** au premier pas : le `{t}` du chemin doit **égaler** le tenant
//! de l'`AuthContext` authentifié — sinon **403**, sans même révéler l'existence d'autres tenants.

use crate::auth::AuthContext;
use crate::project::DecisionStatus;
use crate::registry::{RegistryError, TenantRegistry};
use crate::service::{ApiError, HumanActionInput};
use provenance_dag::HumanAction;
use serde::Serialize;

/// La méthode HTTP (seulement ce dont S2 a besoin).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// Une requête — déjà **authentifiée** (l'`AuthContext` est produit par la coquille en amont). `now` est
/// l'horloge injectée (déterminisme : aucune horloge cachée dans la logique).
#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub auth: AuthContext,
    pub body: Option<String>,
    pub now: u64,
}

/// Une réponse : un code HTTP + un corps JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

#[derive(Serialize)]
struct ErrBody<'a> {
    error: &'a str,
    status: u16,
}

impl Response {
    fn json<T: Serialize>(status: u16, value: &T) -> Response {
        Response { status, body: serde_json::to_string(value).unwrap_or_else(|_| "{}".into()) }
    }
    fn err(status: u16, msg: &str) -> Response {
        Response::json(status, &ErrBody { error: msg, status })
    }
}

/// **Le point d'entrée du routeur.** Pur : aucune E/S, aucune horloge.
pub fn route(reg: &mut TenantRegistry, req: &Request) -> Response {
    let segments: Vec<&str> = req.path.split('/').filter(|s| !s.is_empty()).collect();

    // Toutes les routes sont sous /tenants/{t}/…
    let (t, rest) = match segments.as_slice() {
        ["tenants", t, rest @ ..] => (*t, rest),
        _ => return Response::err(404, "route inconnue"),
    };
    // ISOLATION (§7) : le {t} du chemin DOIT égaler le tenant authentifié.
    if t != req.auth.tenant.as_str() {
        return Response::err(403, "tenant du chemin ≠ tenant authentifié");
    }
    let svc = match reg.resolve(&req.auth) {
        Ok(s) => s,
        Err(RegistryError::UnknownTenant(_)) => return Response::err(404, "tenant inconnu"),
    };

    match (req.method, rest) {
        // GET /tenants/{t}/cases  (?status, ?from, ?to, ?after, ?limit)
        (Method::Get, ["cases"]) => {
            let status = match q(&req.query, "status").map(parse_status) {
                Some(Ok(s)) => Some(s),
                Some(Err(e)) => return Response::err(400, &e),
                None => None,
            };
            let period = match (qn(&req.query, "from"), qn(&req.query, "to")) {
                (Some(f), Some(t2)) => Some((f, t2)),
                (None, None) => None,
                _ => return Response::err(400, "from et to doivent être fournis ensemble"),
            };
            let after_pos = qn_usize(&req.query, "after");
            let limit = qn_usize(&req.query, "limit").unwrap_or(50).clamp(1, 500);
            let q = crate::service::ListQuery { status, period, after_pos, limit };
            match svc.list_cases(&q, &req.auth, req.now) {
                Ok(r) => Response::json(200, &r),
                Err(e) => err_response(e),
            }
        }
        // GET /tenants/{t}/cases/{id}
        (Method::Get, ["cases", id]) => match svc.case_view(id, &req.auth, req.now) {
            Ok(r) => Response::json(200, &r),
            Err(e) => err_response(e),
        },
        // GET /tenants/{t}/cases/{id}/report
        (Method::Get, ["cases", id, "report"]) => match svc.report(id, &req.auth, req.now) {
            Ok(md) => Response { status: 200, body: md },
            Err(e) => err_response(e),
        },
        // POST /tenants/{t}/cases/{id}/human-action
        (Method::Post, ["cases", id, "human-action"]) => {
            let input = match parse_human_action(req.body.as_deref()) {
                Ok(i) => i,
                Err(e) => return Response::err(400, &e),
            };
            match svc.record_human_action(id, &input, &req.auth, req.now) {
                Ok(r) => Response::json(200, &r),
                Err(e) => err_response(e),
            }
        }
        // POST /tenants/{t}/reprove  (re-preuve FROIDE autoritaire — le « vérifier »)
        (Method::Post, ["reprove"]) => match svc.reprove_now(&req.auth, req.now) {
            Ok(env) => Response::json(200, &env),
            Err(e) => err_response(e),
        },
        // GET /tenants/{t}/export  (?cases=a,b,c)
        (Method::Get, ["export"]) => {
            let only: Option<Vec<String>> = q(&req.query, "cases").map(|s| s.split(',').filter(|x| !x.is_empty()).map(|x| x.to_string()).collect());
            match svc.export(only.as_deref(), &req.auth, req.now) {
                Ok(c) => Response::json(200, &c),
                Err(e) => err_response(e),
            }
        }
        _ => Response::err(404, "route inconnue"),
    }
}

fn err_response(e: ApiError) -> Response {
    let status = match e {
        ApiError::NotFound(_) => 404,
        ApiError::Forbidden(_) => 403,
        ApiError::BadRequest(_) => 400,
        ApiError::Integrity(_) => 409,
    };
    Response::err(status, &e.to_string())
}

// ── helpers de query/body ──

fn q<'a>(query: &'a [(String, String)], key: &str) -> Option<&'a str> {
    query.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}
fn qn(query: &[(String, String)], key: &str) -> Option<u64> {
    q(query, key).and_then(|v| v.parse().ok())
}
fn qn_usize(query: &[(String, String)], key: &str) -> Option<usize> {
    q(query, key).and_then(|v| v.parse().ok())
}

fn parse_status(s: &str) -> Result<DecisionStatus, String> {
    match s.to_ascii_uppercase().as_str() {
        "COMPLIANT" => Ok(DecisionStatus::Compliant),
        "FAIL" => Ok(DecisionStatus::Fail),
        "EXCEPTION" => Ok(DecisionStatus::Exception),
        "PENDING" => Ok(DecisionStatus::Pending),
        other => Err(format!("statut inconnu : {other}")),
    }
}

fn parse_action(s: &str) -> Result<HumanAction, String> {
    match s.to_ascii_uppercase().as_str() {
        "APPROVE" => Ok(HumanAction::Approve),
        "OVERRIDE" => Ok(HumanAction::Override),
        "ESCALATE" => Ok(HumanAction::Escalate),
        "COMMENT" => Ok(HumanAction::Comment),
        other => Err(format!("action inconnue : {other}")),
    }
}

#[derive(serde::Deserialize)]
struct HumanActionBody {
    target_step: String,
    action: String,
    justification: String,
}

fn parse_human_action(body: Option<&str>) -> Result<HumanActionInput, String> {
    let raw = body.ok_or("corps JSON manquant")?;
    let b: HumanActionBody = serde_json::from_str(raw).map_err(|e| format!("JSON invalide : {e}"))?;
    Ok(HumanActionInput {
        target_step: b.target_step,
        action: parse_action(&b.action)?,
        justification: b.justification,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Role;
    use crate::project::ProjectionConfig;
    use provenance_capture::LlmCall;
    use provenance_dag::TenantKey;

    fn reg() -> TenantRegistry {
        let mut r = TenantRegistry::new();
        r.provision("acme", TenantKey::derive(b"root", "acme"), ProjectionConfig::default());
        r.provision("globex", TenantKey::derive(b"root", "globex"), ProjectionConfig::default());
        {
            let svc = r.resolve(&AuthContext::new("acme", Role::Admin, "o")).unwrap();
            svc.ingest_llm(&LlmCall { case_id: "SUP-A", step: "sanction", agent_role: "sanction_check", inputs: b"i", model: "phi4", version: "1", output: "PASS", evidence_steps: &[], timestamp: 1 }).unwrap();
            svc.ingest_llm(&LlmCall { case_id: "SUP-A", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"i", model: "phi4", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 2 }).unwrap();
        }
        r
    }

    fn req(method: Method, path: &str, auth: AuthContext) -> Request {
        Request { method, path: path.into(), query: vec![], auth, body: None, now: 100 }
    }

    #[test]
    fn get_case_returns_decision_and_envelope() {
        let mut r = reg();
        let resp = route(&mut r, &req(Method::Get, "/tenants/acme/cases/SUP-A", AuthContext::new("acme", Role::ComplianceOfficer, "o")));
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("\"case_id\":\"SUP-A\""));
        assert!(resp.body.contains("\"provenance\""));
        assert!(resp.body.contains("INTEGRITY_ONLY"));
        assert!(!resp.body.to_lowercase().contains("correct"), "jamais le token « correct »");
    }

    #[test]
    fn cross_tenant_path_is_forbidden() {
        let mut r = reg();
        // authentifié ACME, mais chemin GLOBEX → 403, sans rien révéler.
        let resp = route(&mut r, &req(Method::Get, "/tenants/globex/cases/SUP-A", AuthContext::new("acme", Role::Admin, "o")));
        assert_eq!(resp.status, 403, "le {{t}} du chemin doit égaler le tenant authentifié");
    }

    #[test]
    fn list_filter_and_human_action_flow() {
        let mut r = reg();
        let auth = AuthContext::new("acme", Role::ComplianceOfficer, "officer:hanna");
        // liste
        let resp = route(&mut r, &req(Method::Get, "/tenants/acme/cases", auth.clone()));
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("SUP-A"));
        // action humaine scellée
        let mut hreq = req(Method::Post, "/tenants/acme/cases/SUP-A/human-action", auth.clone());
        hreq.body = Some(r#"{"target_step":"aggregate","action":"APPROVE","justification":"revue conforme"}"#.into());
        let resp = route(&mut r, &hreq);
        assert_eq!(resp.status, 200, "body={}", resp.body);
        assert!(resp.body.contains("SUP-A::human-0"));
        // viewer ne peut pas agir → 403
        let mut vreq = req(Method::Post, "/tenants/acme/cases/SUP-A/human-action", AuthContext::new("acme", Role::Viewer, "v"));
        vreq.body = Some(r#"{"target_step":"aggregate","action":"OVERRIDE","justification":"x"}"#.into());
        assert_eq!(route(&mut r, &vreq).status, 403);
    }

    #[test]
    fn crafted_after_cursor_does_not_panic() {
        // ?after=usize::MAX ne doit ni paniquer (debug) ni boucler : page vide propre, statut 200.
        let mut r = reg();
        let mut q = req(Method::Get, "/tenants/acme/cases", AuthContext::new("acme", Role::ComplianceOfficer, "o"));
        q.query = vec![("after".into(), u64::MAX.to_string())];
        let resp = route(&mut r, &q);
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("\"cases\":[]"), "page vide propre : {}", resp.body);
    }

    #[test]
    fn reprove_and_export_routes() {
        let mut r = reg();
        let auth = AuthContext::new("acme", Role::ComplianceOfficer, "o");
        assert_eq!(route(&mut r, &req(Method::Post, "/tenants/acme/reprove", auth.clone())).status, 200);
        let exp = route(&mut r, &req(Method::Get, "/tenants/acme/export", auth.clone()));
        assert_eq!(exp.status, 200);
        assert!(exp.body.contains("manifest_root"));
        // unknown route
        assert_eq!(route(&mut r, &req(Method::Get, "/tenants/acme/nope", auth)).status, 404);
    }
}
