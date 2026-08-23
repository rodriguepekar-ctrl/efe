// SPDX-License-Identifier: UNLICENSED
//! **Le cœur de la coquille de transport** — `handle(console, req, now)`, **pur** (aucun socket, aucune
//! horloge cachée : `now` est injecté), donc **air-gap-testable** comme le routeur de S2. Il fait trois
//! choses, et **aucune décision de confiance** : (1) résoudre la session → `AuthContext` (401 sinon),
//! (2) router le chemin console vers un appel S2, (3) **rendre** la réponse (HTML pour la console, JSON
//! pour `/api`, un tar pour l'export).
//!
//! L'invariant cardinal (`I-S3-VUE-PURE`) : le passe-plat `/api/tenants/{t}/…` rend **exactement** ce que
//! `audit_api::route()` rendrait — la coquille n'ajoute ni ne retranche aucune logique. Et `I-S3-RBAC-
//! COSMÉTIQUE` : une action/export émise hors UI par un rôle insuffisant est refusée **par S2** (403),
//! quoi qu'affiche l'interface.

use crate::app::Console;
use crate::export::{build_bundle_tar, ExportFail};
use crate::http::{parse_urlencoded, HttpMethod, HttpRequest, HttpResponse};
use crate::render;
use crate::session::extract_token;
use audit_api::{route, ApiError, AuthContext, Cursor, DecisionStatus, ListQuery, Method, Request, Role};
use provenance_dag::HumanAction;
use reg_evidence::{CaseScope, Framework};

/// Le point d'entrée de la coquille. Pur ; `now` est l'horloge injectée (le serveur passe l'heure réelle).
pub fn handle(console: &mut Console, req: &HttpRequest, now: u64) -> HttpResponse {
    // ── 0. M7 — les sondes d'exploitation, AVANT l'authentification ──
    //
    // Un superviseur n'a pas de jeton. Exiger l'authentification rendrait la sonde inutilisable ;
    // lui donner un jeton de service créerait un identifiant de plus à protéger, pour surveiller.
    // Le prix à payer est que ces DEUX réponses sont publiques — elles ne révèlent donc ni nom de
    // tenant, ni compte, ni état d'intégrité : rien qu'un fait binaire sur le processus.
    //
    // C'est la SEULE exception à « tout passe par l'authentification », et le test
    // `seules_les_deux_sondes_sont_publiques` interdit qu'il y en ait une troisième.
    if req.method == HttpMethod::Get {
        let segs: Vec<&str> = req.path.split('/').filter(|s| !s.is_empty()).collect();
        match segs.as_slice() {
            ["healthz"] => return sante(),
            ["readyz"] => return pret(console),
            _ => {}
        }
    }

    // ── 1. authentification : la coquille construit un AuthContext DÉJÀ vérifié, ou 401 ──
    let token = match extract_token(req) {
        Some(t) => t,
        None => return unauth(req),
    };
    let auth = match console.sessions().resolve(&token) {
        Some(a) => a,
        None => return unauth(req),
    };
    let tenant = auth.tenant.as_str().to_string();
    let role = auth.role;

    let segs: Vec<&str> = req.path.split('/').filter(|s| !s.is_empty()).collect();

    match (&req.method, segs.as_slice()) {
        // ── le passe-plat /api : I-S3-VUE-PURE (rend exactement route()) ──
        (_, ["api", rest @ ..]) => api_passthrough(console, &auth, &req.method, rest, req, now),

        // ── la console (HTML) ──
        (HttpMethod::Get, []) => list_view(console, &auth, req, now, false),
        (HttpMethod::Get, ["exceptions"]) => list_view(console, &auth, req, now, true),
        (HttpMethod::Get, ["cases", id]) => decision_view(console, &auth, &tenant, role, id, now),
        (HttpMethod::Post, ["cases", id, "action"]) => action(console, &auth, id, req, now),
        (HttpMethod::Post, ["reprove"]) => reprove(console, &auth, &tenant, role, now),
        (HttpMethod::Get, ["cases", id, "export"]) => export(console, &auth, CaseScope::Case((*id).to_string()), req, now),
        (HttpMethod::Get, ["export"]) => export(console, &auth, CaseScope::FullTenant, req, now),

        // méthode non supportée sur une route connue, ou route inconnue
        (HttpMethod::Other(_), _) => HttpResponse::html(405, render::page("Méthode non supportée", &tenant, role, &render::error_page(405, "Méthode non supportée", "seuls GET et POST", None))),
        _ => HttpResponse::html(404, render::page("Introuvable", &tenant, role, &render::error_page(404, "Route inconnue", &req.path, None))),
    }
}

// ── les vues console ──

fn list_view(console: &mut Console, auth: &AuthContext, req: &HttpRequest, now: u64, exceptions: bool) -> HttpResponse {
    let tenant = auth.tenant.as_str();
    let role = auth.role;
    // Curseur opaque (?after) — un token illisible est une erreur d'usage (400), pas un panic.
    let cursor = match req.query_get("after") {
        Some(tok) => match Cursor::decode(tok) {
            Some(c) => Some(c),
            None => return HttpResponse::html(400, render::page("Erreur", tenant, role, &render::error_page(400, "Curseur invalide", "le lien de pagination est illisible", Some("revenez à la première page")))),
        },
        None => None,
    };
    // Filtres (§7) : statut, période, exigence (rôle). Dans la file d'exceptions, le filtre STATUT est
    // ignoré (c'est déjà le préréglage « non-conforme », appliqué au rendu) ; rôle/période restent honorés.
    let status = if exceptions {
        None
    } else {
        match req.query_get("status").filter(|s| !s.is_empty()) {
            Some(s) => match parse_status_param(s) {
                Some(st) => Some(st),
                None => return HttpResponse::html(400, render::page("Erreur", tenant, role, &render::error_page(400, "Statut inconnu", s, Some("statuts : COMPLIANT, FAIL, EXCEPTION, PENDING")))),
            },
            None => None,
        }
    };
    let from = req.query_get("from").filter(|s| !s.is_empty()).and_then(|s| s.parse::<u64>().ok());
    let to = req.query_get("to").filter(|s| !s.is_empty()).and_then(|s| s.parse::<u64>().ok());
    let period = match (from, to) {
        (Some(f), Some(t)) => Some((f, t)),
        _ => None, // un formulaire de période à demi rempli ne filtre pas (both-or-none)
    };
    let role_filter = req.query_get("role").filter(|s| !s.is_empty()).map(|s| s.to_string());
    let query = ListQuery {
        status,
        period,
        role: role_filter.clone(),
        cursor,
        limit: req.query_get("limit").and_then(|s| s.parse().ok()).unwrap_or(50usize).clamp(1, 500),
    };
    let svc = match console.registry_mut().resolve(auth) {
        Ok(s) => s,
        Err(_) => return not_found(tenant, role),
    };
    match svc.list_cases(&query, auth, now) {
        Ok(list) => {
            let (title, base) = if exceptions { ("File d'exceptions", "/exceptions") } else { ("Dossiers", "/") };
            let filters = render::Filters {
                status: if exceptions { None } else { req.query_get("status").filter(|s| !s.is_empty()).map(str::to_string) },
                role: role_filter,
                from: req.query_get("from").filter(|s| !s.is_empty()).map(str::to_string),
                to: req.query_get("to").filter(|s| !s.is_empty()).map(str::to_string),
            };
            HttpResponse::html(200, render::page(title, tenant, role, &render::cases(&list, exceptions, &filters, base)))
        }
        Err(e) => api_err_html(&e, tenant, role),
    }
}

fn decision_view(console: &mut Console, auth: &AuthContext, tenant: &str, role: Role, id: &str, now: u64) -> HttpResponse {
    let svc = match console.registry_mut().resolve(auth) {
        Ok(s) => s,
        Err(_) => return not_found(tenant, role),
    };
    match svc.case_view(id, auth, now) {
        Ok(resp) => HttpResponse::html(200, render::page(&format!("Dossier {id}"), tenant, role, &render::decision(&resp, role))),
        Err(e) => api_err_html(&e, tenant, role),
    }
}

fn action(console: &mut Console, auth: &AuthContext, id: &str, req: &HttpRequest, now: u64) -> HttpResponse {
    let tenant = auth.tenant.as_str();
    let role = auth.role;
    let body = String::from_utf8_lossy(&req.body);
    let form = parse_urlencoded(&body);
    let get = |k: &str| form.iter().find(|(fk, _)| fk == k).map(|(_, v)| v.clone());
    let (target_step, action_str, justification) = match (get("target_step"), get("action"), get("justification")) {
        (Some(t), Some(a), Some(j)) => (t, a, j),
        _ => return HttpResponse::html(400, render::page("Erreur", tenant, role, &render::error_page(400, "Formulaire incomplet", "target_step, action et justification sont requis", None))),
    };
    let action = match parse_action(&action_str) {
        Some(a) => a,
        None => return HttpResponse::html(400, render::page("Erreur", tenant, role, &render::error_page(400, "Action inconnue", &action_str, None))),
    };
    let input = audit_api::HumanActionInput { target_step, action, justification };
    let svc = match console.registry_mut().resolve(auth) {
        Ok(s) => s,
        Err(_) => return not_found(tenant, role),
    };
    // S2 ENFORCE le RBAC ici (can_act) — le masquage du bouton côté UI n'était que cosmétique.
    match svc.record_human_action(id, &input, auth, now) {
        Ok(_) => HttpResponse::redirect(&format!("/cases/{id}")), // PRG : on relit la décision mise à jour
        Err(e) => api_err_html(&e, tenant, role),
    }
}

fn reprove(console: &mut Console, auth: &AuthContext, tenant: &str, role: Role, now: u64) -> HttpResponse {
    let svc = match console.registry_mut().resolve(auth) {
        Ok(s) => s,
        Err(_) => return not_found(tenant, role),
    };
    match svc.reprove_now(auth, now) {
        Ok(env) => HttpResponse::html(200, render::page("Vérification", tenant, role, &render::reproved(&env))),
        Err(e) => api_err_html(&e, tenant, role),
    }
}

fn export(console: &mut Console, auth: &AuthContext, scope: CaseScope, req: &HttpRequest, now: u64) -> HttpResponse {
    let tenant = auth.tenant.as_str();
    let role = auth.role;
    let framework = parse_framework(req.query_get("fw"));
    match build_bundle_tar(console, auth, &scope, framework, now) {
        Ok(tar) => HttpResponse::download(&tar.filename, "application/x-tar", tar.bytes),
        Err(e) => export_err_html(&e, tenant, role),
    }
}

// ── le passe-plat /api (I-S3-VUE-PURE) ──

fn api_passthrough(console: &mut Console, auth: &AuthContext, method: &HttpMethod, rest: &[&str], req: &HttpRequest, now: u64) -> HttpResponse {
    let m = match method {
        HttpMethod::Get => Method::Get,
        HttpMethod::Post => Method::Post,
        HttpMethod::Other(_) => return HttpResponse::json(405, "{\"error\":\"méthode non supportée\",\"status\":405}".as_bytes().to_vec()),
    };
    // Le chemin S2 est exactement /tenants/{t}/… (ce qui suit /api). route() enforce {t} == tenant authentifié.
    let s2_path = format!("/{}", rest.join("/"));
    let s2_req = Request {
        method: m,
        path: s2_path,
        query: req.query.clone(),
        auth: auth.clone(),
        body: if req.body.is_empty() { None } else { Some(String::from_utf8_lossy(&req.body).to_string()) },
        now,
    };
    let resp = route(console.registry_mut(), &s2_req);
    // route() rend du JSON SAUF la route .../report (Markdown brut, statut 200) → on étiquette correctement
    // le Content-Type. Le corps et le statut restent IDENTIQUES à route() (la parité I-S3-VUE-PURE tient).
    let is_markdown_report = rest.last() == Some(&"report") && resp.status == 200;
    let content_type = if is_markdown_report { "text/markdown; charset=utf-8" } else { "application/json; charset=utf-8" };
    HttpResponse { status: resp.status, content_type: content_type.to_string(), headers: Vec::new(), body: resp.body.into_bytes() }
}

// ── helpers ──

/// `/healthz` — **le processus vit**, et rien d'autre.
///
/// Une sonde de vivacité qui consulterait l'état finirait par faire tuer un processus parfaitement
/// sain dont une dépendance hoquette. Elle répond donc toujours, tant qu'elle peut répondre.
fn sante() -> HttpResponse {
    HttpResponse::json(200, b"{\"status\":\"ok\"}".to_vec())
}

/// `/readyz` — le service est **provisionné**.
///
/// Délibérément **pas** l'intégrité des chaînes, et cette décision mérite d'être écrite ici parce
/// qu'elle paraît contre-intuitive. Le dépôt a déjà tranché ailleurs : `case_view` *« sert même un
/// store cassé — mais avec une enveloppe `ok:false` (échec FORT et VISIBLE) »*. Servir du cassé est
/// **voulu**, et c'est visible là où c'est utile. Or un superviseur qui voit `readyz` rouge
/// **redémarre** — et un redémarrage ne répare pas une chaîne corrompue. On obtiendrait une boucle
/// de redémarrage qui ne corrige rien, et qui masquerait le vrai signal.
///
/// Une sonde de disponibilité ne doit signaler que **ce qu'un redémarrage peut réparer**. Ici : un
/// fichier de déclaration non monté, un secret absent, un log illisible — tout ce qui laisse le
/// service incapable de servir qui que ce soit.
///
/// Ne révèle **ni le nombre de tenants ni leurs noms** : la réponse est publique, et compter les
/// tenants d'un déploiement est déjà un renseignement.
fn pret(console: &Console) -> HttpResponse {
    if console.est_provisionne() {
        HttpResponse::json(200, b"{\"ready\":true}".to_vec())
    } else {
        HttpResponse::json(503, b"{\"ready\":false,\"reason\":\"non provisionne\"}".to_vec())
    }
}

fn unauth(req: &HttpRequest) -> HttpResponse {
    // Pour /api, un JSON ; pour la console, la page « connexion requise ». Aucune donnée révélée.
    if req.path.starts_with("/api/") {
        HttpResponse::json(401, b"{\"error\":\"connexion requise\",\"status\":401}".to_vec())
    } else {
        HttpResponse::html(401, render::login_required())
    }
}

fn not_found(tenant: &str, role: Role) -> HttpResponse {
    HttpResponse::html(404, render::page("Introuvable", tenant, role, &render::error_page(404, "Tenant inconnu", "", None)))
}

/// Mappe une `ApiError` de S2 en page HTML — avec l'UX honnête : **intégrité → rouge fail-loud**,
/// **curseur périmé → « actualiser »**.
fn api_err_html(e: &ApiError, tenant: &str, role: Role) -> HttpResponse {
    let (status, title, hint): (u16, &str, Option<&str>) = match e {
        ApiError::NotFound(_) => (404, "Introuvable", None),
        ApiError::Forbidden(_) => (403, "Action non autorisée pour votre rôle", None),
        ApiError::BadRequest(_) => (400, "Requête invalide", None),
        ApiError::Integrity(_) => (409, "⚠ Intégrité — opération refusée", Some("le record n'est pas authentique ; on n'exporte jamais du non-prouvé")),
        ApiError::CursorStale(_) => (409, "Les données ont évolué", Some("actualisez la page (la pagination est liée à l'état du DAG)")),
    };
    HttpResponse::html(status, render::page("Erreur", tenant, role, &render::error_page(status, title, &e.to_string(), hint)))
}

fn export_err_html(e: &ExportFail, tenant: &str, role: Role) -> HttpResponse {
    let (status, title): (u16, &str) = match e {
        ExportFail::Forbidden(_) => (403, "Export non autorisé pour votre rôle"),
        ExportFail::NotFound(_) => (404, "Introuvable"),
        ExportFail::Integrity(_) => (409, "⚠ Intégrité — export refusé"),
        ExportFail::Build(_) => (500, "Échec de construction du bundle"),
    };
    HttpResponse::html(status, render::page("Erreur d'export", tenant, role, &render::error_page(status, title, &e.to_string(), None)))
}

fn parse_action(s: &str) -> Option<HumanAction> {
    match s.trim().to_ascii_uppercase().as_str() {
        "APPROVE" => Some(HumanAction::Approve),
        "OVERRIDE" => Some(HumanAction::Override),
        "ESCALATE" => Some(HumanAction::Escalate),
        "COMMENT" => Some(HumanAction::Comment),
        _ => None,
    }
}

fn parse_status_param(s: &str) -> Option<DecisionStatus> {
    match s.trim().to_ascii_uppercase().as_str() {
        "COMPLIANT" => Some(DecisionStatus::Compliant),
        "FAIL" => Some(DecisionStatus::Fail),
        "EXCEPTION" => Some(DecisionStatus::Exception),
        "PENDING" => Some(DecisionStatus::Pending),
        _ => None,
    }
}

fn parse_framework(s: Option<&str>) -> Framework {
    match s.map(|x| x.trim().to_ascii_lowercase()).as_deref() {
        Some("gdpr") => Framework::Gdpr,
        Some("csddd") => Framework::Csddd,
        Some("lksg") => Framework::Lksg,
        _ => Framework::EuAiAct, // défaut
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audit_api::{ProjectionConfig, TenantRegistry};
    use provenance_capture::LlmCall;
    use provenance_dag::TenantKey;

    fn console() -> Console {
        let mut c = Console::new();
        c.provision("acme", TenantKey::from_bytes([8u8; 32]));
        c.add_session("offtok", "acme", Role::ComplianceOfficer, "officer:hanna");
        c.add_session("vtok", "acme", Role::Viewer, "v");
        {
            let auth = AuthContext::new("acme", Role::ComplianceOfficer, "o");
            let svc = c.resolve_service(&auth).unwrap();
            svc.ingest_llm(&LlmCall { case_id: "SUP-A", step: "sanction", agent_role: "sanction_check", inputs: b"i", model: "phi4", version: "1", output: "PASS", evidence_steps: &[], timestamp: 1 }).unwrap();
            svc.ingest_llm(&LlmCall { case_id: "SUP-A", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"i", model: "phi4", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 2 }).unwrap();
        }
        c
    }

    fn get(path: &str, token: Option<&str>) -> HttpRequest {
        let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
        HttpRequest::parse(format!("GET {path} HTTP/1.1\r\n{auth}\r\n").as_bytes()).unwrap()
    }

    // ── M7 : les sondes d'exploitation ──

    /// **Le garde de la seule exception à l'authentification.** Deux routes sont publiques ; ce test
    /// interdit qu'il y en ait une troisième. Il balaie tout ce que la console sert, plus quelques
    /// contournements classiques : sans jeton, tout le reste doit rendre 401.
    #[test]
    fn seules_les_deux_sondes_sont_publiques() {
        let mut c = console();
        const PUBLIQUES: [&str; 2] = ["/healthz", "/readyz"];
        const TOUT_LE_RESTE: [&str; 12] = [
            "/",
            "/exceptions",
            "/cases/SUP-A",
            "/export",
            "/cases/SUP-A/export",
            "/api/tenants/acme/cases",
            "/api/tenants/acme/cases/SUP-A",
            "/api/tenants/acme/cases/SUP-A/report",
            "/api/tenants/acme/export",
            // contournements : casse, chemin relatif, préfixe voisin
            "/HEALTHZ",
            "/../healthz",
            "/healthz/../api/tenants/acme/cases",
        ];

        for p in PUBLIQUES {
            let r = handle(&mut c, &get(p, None), 100);
            assert_eq!(r.status, 200, "{p} doit répondre sans jeton");
            assert!(r.content_type.starts_with("application/json"), "{p} parle à une machine");
        }
        for p in TOUT_LE_RESTE {
            let r = handle(&mut c, &get(p, None), 100);
            assert_eq!(r.status, 401, "{p} doit exiger un jeton");
        }
    }

    /// Les sondes ne révèlent **ni nom de tenant, ni compte**. Elles sont publiques : compter les
    /// tenants d'un déploiement est déjà un renseignement.
    #[test]
    fn les_sondes_ne_revelent_rien() {
        let mut c = console();
        for p in ["/healthz", "/readyz"] {
            let corps = String::from_utf8_lossy(&handle(&mut c, &get(p, None), 100).body).to_string();
            assert!(!corps.contains("acme"), "{p} ne doit nommer aucun tenant : {corps}");
            assert!(!corps.contains('1'), "{p} ne doit compter personne : {corps}");
            assert!(corps.len() < 64, "{p} : une sonde répond court");
        }
    }

    /// Une console non provisionnée **vit** (healthz) mais n'est **pas prête** (readyz) — et c'est
    /// exactement ce qu'un redémarrage peut réparer.
    #[test]
    fn une_console_non_provisionnee_vit_mais_nest_pas_prete() {
        let mut vide = Console::new();
        assert_eq!(handle(&mut vide, &get("/healthz", None), 100).status, 200);
        let r = handle(&mut vide, &get("/readyz", None), 100);
        assert_eq!(r.status, 503, "aucun tenant, aucune session : rien à servir");
        assert!(String::from_utf8_lossy(&r.body).contains("non provisionne"));

        // Un tenant SANS session ne suffit pas : personne ne pourrait le lire.
        let mut muet = Console::new();
        muet.provision("acme", TenantKey::from_bytes([1u8; 32]));
        assert_eq!(handle(&mut muet, &get("/readyz", None), 100).status, 503, "illisible = pas prêt");
    }

    #[test]
    fn unauthenticated_is_401_and_reveals_nothing() {
        let mut c = console();
        let resp = handle(&mut c, &get("/cases/SUP-A", None), 100);
        assert_eq!(resp.status, 401);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("Connexion requise"));
        assert!(!body.contains("SUP-A"), "aucune donnée révélée sans session");
    }

    #[test]
    fn officer_sees_cases_and_decision() {
        let mut c = console();
        let list = handle(&mut c, &get("/", Some("offtok")), 100);
        assert_eq!(list.status, 200);
        assert!(String::from_utf8_lossy(&list.body).contains("SUP-A"));
        let dec = handle(&mut c, &get("/cases/SUP-A", Some("offtok")), 100);
        assert_eq!(dec.status, 200);
        let html = String::from_utf8_lossy(&dec.body);
        assert!(html.contains("SUP-A::sanction"), "la décision cite ses nœuds");
        assert!(html.contains("Sceller la revue"), "l'officier voit l'action");
    }

    #[test]
    fn api_passthrough_matches_route_exactly() {
        // I-S3-VUE-PURE : /api/tenants/{t}/… rend EXACTEMENT ce que route() rendrait (parité).
        let mut c = console();
        let via_shell = handle(&mut c, &get("/api/tenants/acme/cases/SUP-A", Some("offtok")), 100);
        // appel direct de route(), avec la même Request logique.
        let mut reg = TenantRegistry::new();
        reg.provision("acme", TenantKey::from_bytes([8u8; 32]), ProjectionConfig::default());
        {
            let auth = AuthContext::new("acme", Role::ComplianceOfficer, "o");
            let svc = reg.resolve(&auth).unwrap();
            svc.ingest_llm(&LlmCall { case_id: "SUP-A", step: "sanction", agent_role: "sanction_check", inputs: b"i", model: "phi4", version: "1", output: "PASS", evidence_steps: &[], timestamp: 1 }).unwrap();
            svc.ingest_llm(&LlmCall { case_id: "SUP-A", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"i", model: "phi4", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 2 }).unwrap();
        }
        let direct = route(&mut reg, &Request { method: Method::Get, path: "/tenants/acme/cases/SUP-A".into(), query: vec![], auth: AuthContext::new("acme", Role::ComplianceOfficer, "officer:hanna"), body: None, now: 100 });
        assert_eq!(via_shell.status, direct.status, "même statut que route()");
        assert_eq!(String::from_utf8_lossy(&via_shell.body), direct.body, "même corps JSON que route() — la coquille n'ajoute aucune décision");
    }

    #[test]
    fn rbac_is_enforced_by_s2_not_the_hidden_button() {
        // I-S3-RBAC-COSMÉTIQUE : un Viewer qui POSTe une action DIRECTEMENT (hors UI) est refusé par S2.
        let mut c = console();
        let body = "target_step=aggregate&action=OVERRIDE&justification=x";
        let raw = format!("POST /cases/SUP-A/action HTTP/1.1\r\nAuthorization: Bearer vtok\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
        let req = HttpRequest::parse(raw.as_bytes()).unwrap();
        let resp = handle(&mut c, &req, 100);
        assert_eq!(resp.status, 403, "S2 refuse l'action d'un Viewer, quoi qu'affiche l'UI");
        // …et la MÊME action via le passe-plat /api est aussi 403 (RBAC enforcé par S2 sur les DEUX surfaces).
        let body2 = r#"{"target_step":"aggregate","action":"OVERRIDE","justification":"x"}"#;
        let raw2 = format!("POST /api/tenants/acme/cases/SUP-A/human-action HTTP/1.1\r\nAuthorization: Bearer vtok\r\nContent-Length: {}\r\n\r\n{}", body2.len(), body2);
        let req2 = HttpRequest::parse(raw2.as_bytes()).unwrap();
        let resp2 = handle(&mut c, &req2, 100);
        assert_eq!(resp2.status, 403, "le Viewer est aussi refusé via /api — la sécurité est dans S2, pas dans l'UI");
    }

    #[test]
    fn api_report_route_is_labelled_markdown_not_json() {
        // I-S3-VUE-PURE, finition : le passe-plat /api étiquette le récit (Markdown) correctement, sans
        // changer le corps/statut de route().
        let mut c = console();
        let resp = handle(&mut c, &get("/api/tenants/acme/cases/SUP-A/report", Some("offtok")), 100);
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "text/markdown; charset=utf-8", "le récit est du Markdown, pas du JSON");
        // les autres routes /api restent du JSON.
        let cases = handle(&mut c, &get("/api/tenants/acme/cases/SUP-A", Some("offtok")), 100);
        assert!(cases.content_type.starts_with("application/json"));
    }

    #[test]
    fn officer_action_seals_and_redirects() {
        let mut c = console();
        let body = "target_step=aggregate&action=APPROVE&justification=revue%20conforme";
        let raw = format!("POST /cases/SUP-A/action HTTP/1.1\r\nAuthorization: Bearer offtok\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
        let req = HttpRequest::parse(raw.as_bytes()).unwrap();
        let resp = handle(&mut c, &req, 100);
        assert_eq!(resp.status, 303, "PRG après l'action scellée");
        assert_eq!(resp.headers.iter().find(|(k, _)| k == "location").map(|(_, v)| v.as_str()), Some("/cases/SUP-A"));
        // l'oversight apparaît désormais dans la décision.
        let dec = handle(&mut c, &get("/cases/SUP-A", Some("offtok")), 101);
        assert!(String::from_utf8_lossy(&dec.body).contains("APPROUVER"), "la revue scellée est tracée");
    }

    #[test]
    fn export_streams_a_tar_download() {
        let mut c = console();
        let resp = handle(&mut c, &get("/cases/SUP-A/export?fw=eu_ai_act", Some("offtok")), 100);
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "application/x-tar");
        assert!(resp.headers.iter().any(|(k, v)| k == "content-disposition" && v.contains("attachment")));
        assert!(!resp.body.is_empty() && resp.body.len() % 512 == 0, "tar bien formé");
        // la clé ne part jamais.
        assert!(!String::from_utf8_lossy(&resp.body).contains(&"08".repeat(32)));
    }

    #[test]
    fn status_and_period_filters_narrow_the_list() {
        // §7 : la console filtre par statut ET par période (pas seulement par rôle). Le formulaire est
        // pré-rempli ; un statut inconnu est une erreur d'usage (400).
        let mut c = Console::new();
        c.provision("acme", TenantKey::from_bytes([8u8; 32]));
        c.add_session("offtok", "acme", Role::ComplianceOfficer, "o");
        {
            let auth = AuthContext::new("acme", Role::ComplianceOfficer, "o");
            let svc = c.resolve_service(&auth).unwrap();
            svc.ingest_llm(&LlmCall { case_id: "SUP-OK", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"i", model: "phi4", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 10 }).unwrap();
            svc.ingest_llm(&LlmCall { case_id: "SUP-BAD", step: "sanction", agent_role: "sanction_check", inputs: b"i", model: "phi4", version: "1", output: "FAIL", evidence_steps: &[], timestamp: 20 }).unwrap();
        }
        // ?status=FAIL → ne garde que SUP-BAD ; le formulaire pré-sélectionne FAIL.
        let r = handle(&mut c, &get("/?status=FAIL", Some("offtok")), 100);
        assert_eq!(r.status, 200);
        let body = String::from_utf8_lossy(&r.body);
        assert!(body.contains("SUP-BAD") && !body.contains("SUP-OK"), "filtre statut FAIL");
        assert!(body.contains("Recherche &amp; filtre"), "le formulaire de filtre apparaît");
        assert!(body.contains("value=\"FAIL\" selected"), "le statut FAIL est pré-sélectionné");
        // ?from/?to (période) sur last_ts : [15,25] garde SUP-BAD (ts 20), pas SUP-OK (ts 10).
        let p = handle(&mut c, &get("/?from=15&to=25", Some("offtok")), 100);
        let pbody = String::from_utf8_lossy(&p.body);
        assert!(pbody.contains("SUP-BAD") && !pbody.contains("SUP-OK"), "filtre période");
        // statut inconnu → 400 (erreur d'usage lisible).
        assert_eq!(handle(&mut c, &get("/?status=BOGUS", Some("offtok")), 100).status, 400);
    }

    #[test]
    fn stale_cursor_renders_actualiser_not_a_silent_page() {
        // I-S3-CURSEUR-HONNÊTE : un curseur émis contre un root antérieur (le DAG a avancé) → 409
        // « actualiser », jamais une page incohérente en silence.
        let mut c = console();
        {
            let auth = AuthContext::new("acme", Role::ComplianceOfficer, "o");
            let svc = c.resolve_service(&auth).unwrap();
            svc.ingest_llm(&LlmCall { case_id: "SUP-B", step: "sanction", agent_role: "sanction_check", inputs: b"i", model: "phi4", version: "1", output: "PASS", evidence_steps: &[], timestamp: 3 }).unwrap();
        }
        // page 1 (limit=1) → un lien « page suivante » porteur d'un curseur.
        let p1 = handle(&mut c, &get("/?limit=1", Some("offtok")), 100);
        let b1 = String::from_utf8_lossy(&p1.body).to_string();
        let cur = b1.split("after=").nth(1).and_then(|s| s.split('"').next()).expect("un curseur dans le lien").to_string();
        // le DAG avance (action humaine scellée) → le root change.
        let pb = "target_step=aggregate&action=APPROVE&justification=x";
        let raw = format!("POST /cases/SUP-A/action HTTP/1.1\r\nAuthorization: Bearer offtok\r\nContent-Length: {}\r\n\r\n{}", pb.len(), pb);
        assert_eq!(handle(&mut c, &HttpRequest::parse(raw.as_bytes()).unwrap(), 101).status, 303);
        // rejouer le curseur périmé → 409, et l'UX dit « actualiser » (pas une page muette).
        let stale = handle(&mut c, &get(&format!("/?after={cur}"), Some("offtok")), 102);
        assert_eq!(stale.status, 409, "curseur périmé → 409");
        assert!(String::from_utf8_lossy(&stale.body).to_lowercase().contains("actualis"), "l'UX dit « actualiser »");
    }
}
