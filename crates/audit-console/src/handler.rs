// SPDX-License-Identifier: UNLICENSED
//! **Le cœur de la coquille de transport** — `handle(console, req, now, &LOCAL)`, **pur** (aucun socket, aucune
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

use crate::canal::{Canal, Plancher};
use crate::session::Provenance;

/// Le point d'entrée de la coquille. Pur ; `now` est l'horloge injectée (le serveur passe l'heure réelle).
pub fn handle(console: &mut Console, req: &HttpRequest, now: u64, canal: &Canal) -> HttpResponse {
    // ── 0. M7 — les sondes d'exploitation, AVANT l'authentification ──
    //
    // Un superviseur n'a pas de jeton. Exiger l'authentification rendrait la sonde inutilisable ;
    // lui donner un jeton de service créerait un identifiant de plus à protéger, pour surveiller.
    // Le prix à payer est que ces DEUX réponses sont publiques — elles ne révèlent donc ni nom de
    // tenant, ni compte, ni état d'intégrité : rien qu'un fait binaire sur le processus.
    //
    // C'est la SEULE exception à « tout passe par l'authentification », et le test
    // `seules_les_deux_sondes_sont_publiques` interdit qu'il y en ait une troisième.
    //
    // **Un SEUL découpage**, pour les deux tables *et* pour le plancher. Il y en avait deux : la
    // même expression, écrite ici et une seconde fois avant le dispatch authentifié. Elles ne
    // pouvaient pas diverger — mais rien n'empêchait d'en modifier UNE (percent-décodage, barre
    // finale, normalisation), et le plancher de l'étape 3 aurait alors cessé, en silence, de
    // porter sur ce que l'étape 4 dispatche. Un calcul, pas deux qu'un test compare.
    let segs: Vec<&str> = req.path.split('/').filter(|s| !s.is_empty()).collect();
    match (&req.method, segs.as_slice()) {
        (HttpMethod::Get, ["healthz"]) => return sante(),
        (HttpMethod::Get, ["readyz"]) => return pret(console),
        // M8 — la CONNEXION doit être atteignable sans être connecté. C'est la seule raison pour
        // laquelle ces routes précèdent l'authentification, et le garde
        // `seules_les_routes_publiques_declarees_le_sont` interdit qu'il y en ait une sixième.
        (HttpMethod::Get, ["login"]) => return formulaire_connexion(None),
        (HttpMethod::Post, ["login"]) => return connexion(console, req, now, canal),
        (HttpMethod::Post, ["logout"]) => return deconnexion(),
        _ => {}
    }

    // ── 1. authentification : la coquille construit un AuthContext DÉJÀ vérifié, ou 401 ──
    let (token, provenance) = match extract_token(req) {
        Some(t) => t,
        None => return unauth(req),
    };
    let auth = match console.sessions().resolve(&token, now) {
        Some(a) => a,
        None => return unauth(req),
    };

    // ── 2. M8 — CSRF : une requête qui CHANGE l'état et qui s'authentifie par COOKIE doit prouver
    //    son origine. Un cookie part TOUT SEUL, y compris sur une requête déclenchée par un autre
    //    site ; un en-tête `Authorization` ne le peut pas — il est donc lui-même la défense.
    //    Sans cela, un site tiers ferait sceller une approbation sous l'identité d'un officier qui
    //    n'a rien fait, et le record serait indistinguable d'une vraie.
    if req.method == HttpMethod::Post {
        if let Err(refus) = origine_sure(req, provenance == Provenance::EnTete) {
            return refus;
        }
    }

    // ── 3. M8 — le PLANCHER DE CANAL, point de décision unique pour les actes scellés. ──
    if scelle_un_acte(&req.method, &segs) && !console.plancher().franchi_par(canal) {
        return refus_canal(req, console.plancher(), canal);
    }
    let tenant = auth.tenant.as_str().to_string();
    let role = auth.role;

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

/// **Les routes PUBLIQUES** — les seules atteignables sans session, déclarées en un seul endroit.
///
/// Toute autre route rend 401. Cette liste est le pendant exact du `match` qui ouvre [`handle`] :
/// le garde `seules_les_routes_publiques_declarees_le_sont` compte les arms de ce `match` et exige
/// qu'ils soient aussi nombreux — une sixième route publique ajoutée sans être déclarée ici fait
/// tomber le test.
///
/// Chacune se justifie séparément, et aucune ne révèle quoi que ce soit :
/// * les deux **sondes** — un superviseur n'a pas de jeton, et lui en donner un créerait un
///   identifiant de plus à protéger, pour surveiller ;
/// * les trois de **session** — se connecter ne peut pas exiger d'être connecté, et se déconnecter
///   ne doit jamais être empêché par une session déjà cassée.
pub const PUBLIQUES: &[(&str, &str)] = &[
    ("GET", "/healthz"),
    ("GET", "/readyz"),
    ("GET", "/login"),
    ("POST", "/login"),
    ("POST", "/logout"),
];

// ── M8 : la défense CSRF, le plancher de canal, et la connexion ──

/// Cette requête **scellerait-elle** un acte de supervision ?
///
/// **Point de décision unique.** Deux routes atteignent `record_human_action` : le passe-plat
/// `/api/tenants/{t}/cases/{id}/human-action` et le formulaire `/cases/{id}/action`. Les tenir ici,
/// en un seul endroit, est **nécessaire** pour qu'une troisième ne puisse pas apparaître sans
/// plancher — et ce n'est pas suffisant : rien, dans ce fichier, n'oblige qui ajoute une route à
/// venir la déclarer ici.
///
/// C'est le travail du garde [`tests::le_predicat_de_scellement_connait_toutes_les_routes_scellantes`],
/// qui joue **chaque** route du dispatch sous un canal non asserti et vérifie que le magasin scellé
/// ne grandit pas. Cette phrase-ci a longtemps affirmé que ce garde existait ; il n'existait pas —
/// sa seule occurrence dans le dépôt était la promesse elle-même.
fn scelle_un_acte(m: &HttpMethod, segs: &[&str]) -> bool {
    if *m != HttpMethod::Post {
        return false;
    }
    matches!(segs, ["cases", _, "action"])
        || matches!(segs, ["api", "tenants", _, "cases", _, "human-action"])
}

/// Le refus de canal — un 403 qui dit **ce qui manque** et **quel geste corrige**.
fn refus_canal(req: &HttpRequest, plancher: Plancher, canal: &Canal) -> HttpResponse {
    let msg = plancher.refus(canal);
    if req.path.starts_with("/api/") {
        HttpResponse::json(
            403,
            format!(
                "{{\"error\":{},\"status\":403,\"canal\":\"{}\"}}",
                json_str(&msg),
                canal.code()
            )
            .into_bytes(),
        )
    } else {
        HttpResponse::html(
            403,
            render::page("Canal insuffisant", "", audit_api::Role::Viewer, &render::error_page(403, "Canal insuffisant pour sceller", &msg, None)),
        )
    }
}

/// Échappe une chaîne pour un littéral JSON — la coquille n'a pas serde côté réponse d'erreur.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// **La défense CSRF.** `dispense` vaut pour un jeton présenté en **en-tête** : un site tiers ne peut
/// pas faire poser `Authorization` par un navigateur sur une requête simple, donc l'en-tête est
/// lui-même la preuve d'intention. Un **cookie**, lui, part tout seul.
///
/// Sans preuve d'origine, on refuse. C'est volontairement strict : la seule chose qu'un client
/// légitime perd est de devoir envoyer `Origin` (que tout navigateur envoie sur POST depuis des
/// années) ou d'utiliser l'en-tête porteur (que fait tout client machine).
fn origine_sure(req: &HttpRequest, dispense: bool) -> Result<(), HttpResponse> {
    if dispense {
        return Ok(());
    }
    // `Sec-Fetch-Site` est posé par le navigateur lui-même et n'est pas falsifiable par une page.
    if let Some(sfs) = req.header("sec-fetch-site") {
        return match sfs.trim() {
            "same-origin" | "none" => Ok(()),
            autre => Err(refus_csrf(req, &format!("Sec-Fetch-Site: {autre}"))),
        };
    }
    // Sinon `Origin`, comparé à `Host`. La comparaison est une comparaison d'AUTORITÉ : elle passe
    // par l'analyseur unique de M1, jamais par un `split` local (ce serait un quatrième analyseur).
    match (req.header("origin"), req.header("host")) {
        (Some(o), Some(h)) => {
            let a = scella_authority::authority_of(o.trim()).map(|a| a.host().to_string());
            let b = scella_authority::authority_of(&format!("http://{}", h.trim()))
                .map(|a| a.host().to_string());
            match (a, b) {
                (Some(x), Some(y)) if x == y => Ok(()),
                _ => Err(refus_csrf(req, "Origin ne correspond pas à Host")),
            }
        }
        _ => Err(refus_csrf(req, "ni Sec-Fetch-Site ni Origin")),
    }
}

fn refus_csrf(req: &HttpRequest, pourquoi: &str) -> HttpResponse {
    let msg = format!(
        "requête inter-site refusée ({pourquoi}). Une requête authentifiée par COOKIE et qui change \
         l'état doit prouver son origine : sans cela, un autre site ferait sceller un acte de \
         supervision sous une identité qui n'a rien demandé — et le record serait indistinguable \
         d'un vrai. Un client machine emploie « Authorization: Bearer », qui est dispensé."
    );
    if req.path.starts_with("/api/") {
        HttpResponse::json(403, format!("{{\"error\":{},\"status\":403}}", json_str(&msg)).into_bytes())
    } else {
        HttpResponse::html(403, render::page("Requête refusée", "", audit_api::Role::Viewer, &render::error_page(403, "Requête inter-site", &msg, None)))
    }
}

/// `GET /login` — le formulaire. Public, par nécessité.
fn formulaire_connexion(erreur: Option<&str>) -> HttpResponse {
    let alerte = erreur
        .map(|e| format!("<p style=\"color:#b00\">{}</p>", html_echappe(e)))
        .unwrap_or_default();
    HttpResponse::html(
        200,
        format!(
            "<!doctype html><meta charset=\"utf-8\"><title>Connexion</title>\
             <body style=\"font-family:system-ui;max-width:32rem;margin:4rem auto\">\
             <h1>Console de conformité</h1>{alerte}\
             <form method=\"post\" action=\"/login\">\
             <p><label>Jeton de session<br><input name=\"token\" type=\"password\" style=\"width:100%\" autofocus></label></p>\
             <button>Ouvrir la session</button></form>\
             <p style=\"color:#666;font-size:.9em\">Le jeton n'est jamais conservé en clair : seule son empreinte l'est.</p>"
        ),
    )
}

/// `POST /login` — échange un jeton contre un cookie de session.
///
/// Le cookie porte `HttpOnly` (aucun script ne le lit), `SameSite=Strict` (aucun site tiers ne le
/// fait partir) et `Secure` **dès que le canal n'est pas la boucle locale** — poser `Secure` sur du
/// `http://127.0.0.1` empêcherait purement le navigateur d'enregistrer le cookie, donc casserait la
/// console locale. C'est le barreau de canal qui tranche, et c'est sa deuxième utilité.
fn connexion(console: &mut Console, req: &HttpRequest, now: u64, canal: &Canal) -> HttpResponse {
    // Une connexion CHANGE l'état du navigateur : elle exige une preuve d'origine, sans dispense —
    // une connexion forcée depuis un autre site ferait travailler l'officier dans la session de
    // l'attaquant, et les actes qu'il scellerait iraient chez lui.
    if let Err(refus) = origine_sure(req, false) {
        return refus;
    }
    let body = String::from_utf8_lossy(&req.body);
    let form = parse_urlencoded(&body);
    let token = match form.iter().find(|(k, _)| k == "token").map(|(_, v)| v.clone()) {
        Some(t) if !t.is_empty() => t,
        _ => return formulaire_connexion(Some("jeton manquant")),
    };
    if console.sessions().resolve(&token, now).is_none() {
        // Aucune distinction entre « inconnu » et « échu » : la faire apprendrait à un porteur non
        // authentifié que ce jeton a existé.
        return formulaire_connexion(Some("jeton refusé"));
    }
    let secure = if matches!(canal, Canal::A1LoopbackTemoin) { "" } else { "; Secure" };
    let mut r = HttpResponse::html(303, "");
    r.headers.push(("Location".into(), "/".into()));
    r.headers.push((
        "Set-Cookie".into(),
        format!("session={token}; HttpOnly; SameSite=Strict; Path=/{secure}"),
    ));
    r
}

/// `POST /logout` — efface le cookie. Public : se déconnecter ne doit jamais exiger d'être connecté.
fn deconnexion() -> HttpResponse {
    let mut r = HttpResponse::html(303, "");
    r.headers.push(("Location".into(), "/login".into()));
    r.headers.push((
        "Set-Cookie".into(),
        "session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0".into(),
    ));
    r
}

fn html_echappe(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
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
    use crate::canal::Canal;
    use provenance_capture::LlmCall;
    use provenance_dag::TenantKey;

    const LOCAL: Canal = Canal::A1LoopbackTemoin;

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

    /// **Le garde des routes publiques.** La version de M7 énumérait deux chemins et vérifiait que
    /// d'autres étaient protégés — mais elle ne voyait PAS l'apparition d'une troisième route
    /// publique. M8 en a ajouté trois, et elle est restée verte : un garde qui ne peut pas tomber
    /// quand la propriété se casse ne garde rien.
    ///
    /// Il repose désormais sur [`PUBLIQUES`], la liste employée par le dispatch lui-même, et il
    /// compte les arms du `match` d'entrée pour qu'aucune route ne devienne publique sans y figurer.
    #[test]
    fn seules_les_routes_publiques_declarees_le_sont() {
        let mut c = console();

        // (1) Le nombre d'arms pré-authentification doit égaler la liste déclarée.
        let src = include_str!("handler.rs");
        let entree = &src[src.find("pub fn handle(").expect("handle")..];
        let entree = &entree[..entree.find("// ── 1. authentification").expect("frontière")];
        let code: String =
            entree.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        let arms = code.matches("=> return ").count();
        assert_eq!(
            arms,
            PUBLIQUES.len(),
            "le dispatch pré-authentification a {arms} routes et PUBLIQUES en déclare {} : toute \
             route rendue publique doit être déclarée, sinon personne ne la relit",
            PUBLIQUES.len()
        );

        // (2) Chaque route déclarée répond SANS jeton.
        for (m, p) in PUBLIQUES {
            let r = if *m == "GET" { handle(&mut c, &get(p, None), 100, &LOCAL) } else {
                let req = HttpRequest::parse(
                    format!("POST {p} HTTP/1.1\r\nSec-Fetch-Site: same-origin\r\nContent-Length: 0\r\n\r\n").as_bytes(),
                ).expect("requête");
                handle(&mut c, &req, 100, &LOCAL)
            };
            assert_ne!(r.status, 401, "{m} {p} est déclarée publique : elle ne doit pas exiger de jeton");
        }

        // (3) Et tout le reste exige un jeton.
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

        for p in TOUT_LE_RESTE {
            let r = handle(&mut c, &get(p, None), 100, &LOCAL);
            assert_eq!(r.status, 401, "{p} doit exiger un jeton");
        }
    }

    /// Les routes atteignables **après** authentification : une par arm du dispatch, avec un chemin
    /// concret qui l'atteint, et si elle **scelle** un acte humain.
    ///
    /// [`PUBLIQUES`] fait le même travail pour l'autre table, mais elle est *publique* parce que le
    /// dispatch s'en sert. Celle-ci reste ici : c'est un inventaire de garde, pas une déclaration
    /// servie — la publier laisserait croire qu'elle décide de quelque chose.
    const ROUTES_AUTHENTIFIEES: &[(&str, &str, bool)] = &[
        ("POST", "/api/tenants/acme/cases/SUP-A/human-action", true),
        ("GET", "/", false),
        ("GET", "/exceptions", false),
        ("GET", "/cases/SUP-A", false),
        ("POST", "/cases/SUP-A/action", true),
        ("POST", "/reprove", false),
        ("GET", "/cases/SUP-A/export", false),
        ("GET", "/export", false),
    ];

    /// Le nombre d'enregistrements **scellés** du tenant.
    ///
    /// `record_human_action` est « la SEULE écriture » de S2 (`audit-api/src/service.rs`). Ce compte
    /// est donc un détecteur de scellement qui n'a pas besoin de savoir QUOI a été scellé : une
    /// route qui ferait apparaître un nœud d'une autre espèce serait attrapée tout autant.
    fn scelles(c: &mut Console) -> usize {
        c.service_de("acme").expect("acme est provisionné").store().records().len()
    }

    /// Une requête d'officier authentifiée par **en-tête**, donc dispensée de la preuve d'origine
    /// CSRF : ce garde mesure le plancher de canal, pas la défense inter-site, et les faire porter
    /// par le même test rendrait un échec ambigu.
    fn requete_officier(methode: &str, chemin: &str, etape: &str) -> HttpRequest {
        if methode == "GET" {
            return get(chemin, Some("offtok"));
        }
        let corps = if chemin.starts_with("/api/") {
            format!("{{\"target_step\":\"{etape}\",\"action\":\"APPROVE\",\"justification\":\"garde\"}}")
        } else {
            format!("target_step={etape}&action=APPROVE&justification=garde")
        };
        HttpRequest::parse(
            format!(
                "POST {chemin} HTTP/1.1\r\nAuthorization: Bearer offtok\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{corps}",
                corps.len()
            )
            .as_bytes(),
        )
        .expect("requête")
    }

    /// **Le garde du plancher de scellement.**
    ///
    /// La doc de [`scelle_un_acte`] le nommait, et affirmait qu'il « compte les arms des deux tables
    /// de routage pour que ce commentaire ne devienne pas faux ». **Il n'existait pas** : sa seule
    /// occurrence dans le dépôt était la phrase qui le promettait. Le prédicat est pourtant ce qui
    /// rend le scellement de M8 opposable — une troisième route vers `record_human_action` scellerait
    /// un acte humain sur un canal dont personne ne pourrait dire d'où il venait, en silence, et le
    /// nœud produit serait indistinguable d'un nœud légitime.
    ///
    /// Trois parties, et il faut les trois :
    ///
    /// 1. **Le compte.** Le dispatch authentifié a N arms, [`ROUTES_AUTHENTIFIEES`] en déclare N. Une
    ///    route ajoutée sans être inventoriée fait tomber ce test *avant* de servir. Sans cette
    ///    partie, (2) ne jouerait jamais la route neuve — un garde qui ne regarde que ce qu'on a bien
    ///    voulu lui montrer.
    /// 2. **Le falsificateur.** Chaque route inventoriée est jouée sous `NonAssertee` — rang 0, sous
    ///    *tout* plancher — avec une session d'officier **valide**, pour que le seul obstacle
    ///    possible soit le plancher. Le magasin scellé ne doit pas grandir.
    /// 3. **Le contre-témoin.** Les deux routes déclarées scellantes, jouées au-dessus du plancher,
    ///    **doivent** faire grandir le magasin. Sans lui, (2) serait vert dans le monde où plus rien
    ///    ne scelle du tout : c'est exactement l'erreur du garde de M7, qui ne pouvait pas tomber.
    ///
    /// **Il a été contre-prouvé, pas seulement écrit.** Une troisième route
    /// `POST /cases/{id}/approuver` branchée sur `action` et absente de [`scelle_un_acte`] :
    /// non inventoriée, (1) tombe (9 arms contre 8) ; inventoriée, (2) tombe en montrant
    /// **HTTP 303** — la route avait *réussi*, elle avait scellé un acte de supervision
    /// (2 enregistrements → 3) sur un canal non asserti. C'est exactement ce que le dépôt aurait
    /// livré en silence.
    ///
    /// Ce qu'il ne prouve **pas** : qu'aucun chemin *hors* de ces deux tables n'atteigne le magasin.
    /// Il n'y en a pas aujourd'hui — S2 n'expose qu'une écriture — mais c'est une lecture du code,
    /// pas une preuve, et il fallait le dire plutôt que de le laisser croire.
    #[test]
    fn le_predicat_de_scellement_connait_toutes_les_routes_scellantes() {
        // ── (1) le compte ──
        let src = include_str!("handler.rs");
        let table = &src[src.find("// ── le passe-plat /api").expect("début de la table")..];
        let table = &table[..table.find("// méthode non supportée sur une route connue").expect("fin de la table")];
        let arms = table.matches(") => ").count();
        assert_eq!(
            arms,
            ROUTES_AUTHENTIFIEES.len(),
            "le dispatch authentifié a {arms} routes, l'inventaire en déclare {} : une route ajoutée \
             doit être inventoriée ICI, sinon elle n'est jamais jouée sous un canal nu et le plancher \
             ne la protège que si quelqu'un y a pensé",
            ROUTES_AUTHENTIFIEES.len()
        );

        // ── (2) le falsificateur : sous un canal NON ASSERTI, rien ne scelle ──
        for (m, p, _) in ROUTES_AUTHENTIFIEES {
            let mut c = console();
            let avant = scelles(&mut c);
            let r = handle(&mut c, &requete_officier(m, p, "aggregate"), 100, &Canal::NonAssertee);
            assert_eq!(
                scelles(&mut c),
                avant,
                "{m} {p} a fait GRANDIR le magasin scellé sous un canal non asserti (HTTP {}) : S2 \
                 n'a qu'une écriture, donc cette croissance EST un acte de supervision scellé sur un \
                 canal que le processus n'a pas pu qualifier",
                r.status
            );
        }

        // ── (3) le contre-témoin : au-dessus du plancher, les scellantes SCELLENT ──
        //
        // Deux étapes DISTINCTES : `node_id` vaut `{cas}::{étape}`, et sceller deux fois la même
        // collisionnerait (`DuplicateId`). Le second appel échouerait alors pour une raison qui n'a
        // rien à voir avec le canal, et le garde passerait pour de mauvaises raisons.
        let etapes = ["aggregate", "sanction"];
        let mut vus = 0;
        for (m, p, scelle) in ROUTES_AUTHENTIFIEES.iter().filter(|(_, _, s)| *s) {
            let mut c = console();
            let avant = scelles(&mut c);
            let r = handle(&mut c, &requete_officier(m, p, etapes[vus]), 100, &LOCAL);
            assert!(
                scelles(&mut c) > avant,
                "{m} {p} est déclarée scellante ({scelle}) mais n'a RIEN scellé au-dessus du plancher \
                 (HTTP {}) : sans cette contre-preuve, l'étape (2) serait verte dans un dépôt où plus \
                 rien ne scelle jamais",
                r.status
            );
            vus += 1;
        }
        assert_eq!(vus, 2, "l'inventaire déclare {vus} routes scellantes ; scelle_un_acte en a deux");
    }

    #[test]
    fn les_sondes_parlent_a_une_machine() {
        let mut c = console();
        for p in ["/healthz", "/readyz"] {
            let r = handle(&mut c, &get(p, None), 100, &LOCAL);
            assert_eq!(r.status, 200);
            assert!(r.content_type.starts_with("application/json"), "{p} parle à une machine");
        }
    }

    /// Les sondes ne révèlent **ni nom de tenant, ni compte**. Elles sont publiques : compter les
    /// tenants d'un déploiement est déjà un renseignement.
    #[test]
    fn les_sondes_ne_revelent_rien() {
        let mut c = console();
        for p in ["/healthz", "/readyz"] {
            let corps = String::from_utf8_lossy(&handle(&mut c, &get(p, None), 100, &LOCAL).body).to_string();
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
        assert_eq!(handle(&mut vide, &get("/healthz", None), 100, &LOCAL).status, 200);
        let r = handle(&mut vide, &get("/readyz", None), 100, &LOCAL);
        assert_eq!(r.status, 503, "aucun tenant, aucune session : rien à servir");
        assert!(String::from_utf8_lossy(&r.body).contains("non provisionne"));

        // Un tenant SANS session ne suffit pas : personne ne pourrait le lire.
        let mut muet = Console::new();
        muet.provision("acme", TenantKey::from_bytes([1u8; 32]));
        assert_eq!(handle(&mut muet, &get("/readyz", None), 100, &LOCAL).status, 503, "illisible = pas prêt");
    }

    #[test]
    fn unauthenticated_is_401_and_reveals_nothing() {
        let mut c = console();
        let resp = handle(&mut c, &get("/cases/SUP-A", None), 100, &LOCAL);
        assert_eq!(resp.status, 401);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("Connexion requise"));
        assert!(!body.contains("SUP-A"), "aucune donnée révélée sans session");
    }

    #[test]
    fn officer_sees_cases_and_decision() {
        let mut c = console();
        let list = handle(&mut c, &get("/", Some("offtok")), 100, &LOCAL);
        assert_eq!(list.status, 200);
        assert!(String::from_utf8_lossy(&list.body).contains("SUP-A"));
        let dec = handle(&mut c, &get("/cases/SUP-A", Some("offtok")), 100, &LOCAL);
        assert_eq!(dec.status, 200);
        let html = String::from_utf8_lossy(&dec.body);
        assert!(html.contains("SUP-A::sanction"), "la décision cite ses nœuds");
        assert!(html.contains("Sceller la revue"), "l'officier voit l'action");
    }

    #[test]
    fn api_passthrough_matches_route_exactly() {
        // I-S3-VUE-PURE : /api/tenants/{t}/… rend EXACTEMENT ce que route() rendrait (parité).
        let mut c = console();
        let via_shell = handle(&mut c, &get("/api/tenants/acme/cases/SUP-A", Some("offtok")), 100, &LOCAL);
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
        let resp = handle(&mut c, &req, 100, &LOCAL);
        assert_eq!(resp.status, 403, "S2 refuse l'action d'un Viewer, quoi qu'affiche l'UI");
        // …et la MÊME action via le passe-plat /api est aussi 403 (RBAC enforcé par S2 sur les DEUX surfaces).
        let body2 = r#"{"target_step":"aggregate","action":"OVERRIDE","justification":"x"}"#;
        let raw2 = format!("POST /api/tenants/acme/cases/SUP-A/human-action HTTP/1.1\r\nAuthorization: Bearer vtok\r\nContent-Length: {}\r\n\r\n{}", body2.len(), body2);
        let req2 = HttpRequest::parse(raw2.as_bytes()).unwrap();
        let resp2 = handle(&mut c, &req2, 100, &LOCAL);
        assert_eq!(resp2.status, 403, "le Viewer est aussi refusé via /api — la sécurité est dans S2, pas dans l'UI");
    }

    #[test]
    fn api_report_route_is_labelled_markdown_not_json() {
        // I-S3-VUE-PURE, finition : le passe-plat /api étiquette le récit (Markdown) correctement, sans
        // changer le corps/statut de route().
        let mut c = console();
        let resp = handle(&mut c, &get("/api/tenants/acme/cases/SUP-A/report", Some("offtok")), 100, &LOCAL);
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "text/markdown; charset=utf-8", "le récit est du Markdown, pas du JSON");
        // les autres routes /api restent du JSON.
        let cases = handle(&mut c, &get("/api/tenants/acme/cases/SUP-A", Some("offtok")), 100, &LOCAL);
        assert!(cases.content_type.starts_with("application/json"));
    }

    #[test]
    fn officer_action_seals_and_redirects() {
        let mut c = console();
        let body = "target_step=aggregate&action=APPROVE&justification=revue%20conforme";
        let raw = format!("POST /cases/SUP-A/action HTTP/1.1\r\nAuthorization: Bearer offtok\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
        let req = HttpRequest::parse(raw.as_bytes()).unwrap();
        let resp = handle(&mut c, &req, 100, &LOCAL);
        assert_eq!(resp.status, 303, "PRG après l'action scellée");
        assert_eq!(resp.headers.iter().find(|(k, _)| k == "location").map(|(_, v)| v.as_str()), Some("/cases/SUP-A"));
        // l'oversight apparaît désormais dans la décision.
        let dec = handle(&mut c, &get("/cases/SUP-A", Some("offtok")), 101, &LOCAL);
        assert!(String::from_utf8_lossy(&dec.body).contains("APPROUVER"), "la revue scellée est tracée");
    }

    #[test]
    fn export_streams_a_tar_download() {
        let mut c = console();
        let resp = handle(&mut c, &get("/cases/SUP-A/export?fw=eu_ai_act", Some("offtok")), 100, &LOCAL);
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
        let r = handle(&mut c, &get("/?status=FAIL", Some("offtok")), 100, &LOCAL);
        assert_eq!(r.status, 200);
        let body = String::from_utf8_lossy(&r.body);
        assert!(body.contains("SUP-BAD") && !body.contains("SUP-OK"), "filtre statut FAIL");
        assert!(body.contains("Recherche &amp; filtre"), "le formulaire de filtre apparaît");
        assert!(body.contains("value=\"FAIL\" selected"), "le statut FAIL est pré-sélectionné");
        // ?from/?to (période) sur last_ts : [15,25] garde SUP-BAD (ts 20), pas SUP-OK (ts 10).
        let p = handle(&mut c, &get("/?from=15&to=25", Some("offtok")), 100, &LOCAL);
        let pbody = String::from_utf8_lossy(&p.body);
        assert!(pbody.contains("SUP-BAD") && !pbody.contains("SUP-OK"), "filtre période");
        // statut inconnu → 400 (erreur d'usage lisible).
        assert_eq!(handle(&mut c, &get("/?status=BOGUS", Some("offtok")), 100, &LOCAL).status, 400);
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
        let p1 = handle(&mut c, &get("/?limit=1", Some("offtok")), 100, &LOCAL);
        let b1 = String::from_utf8_lossy(&p1.body).to_string();
        let cur = b1.split("after=").nth(1).and_then(|s| s.split('"').next()).expect("un curseur dans le lien").to_string();
        // le DAG avance (action humaine scellée) → le root change.
        let pb = "target_step=aggregate&action=APPROVE&justification=x";
        let raw = format!("POST /cases/SUP-A/action HTTP/1.1\r\nAuthorization: Bearer offtok\r\nContent-Length: {}\r\n\r\n{}", pb.len(), pb);
        assert_eq!(handle(&mut c, &HttpRequest::parse(raw.as_bytes()).unwrap(), 101, &LOCAL).status, 303);
        // rejouer le curseur périmé → 409, et l'UX dit « actualiser » (pas une page muette).
        let stale = handle(&mut c, &get(&format!("/?after={cur}"), Some("offtok")), 102, &LOCAL);
        assert_eq!(stale.status, 409, "curseur périmé → 409");
        assert!(String::from_utf8_lossy(&stale.body).to_lowercase().contains("actualis"), "l'UX dit « actualiser »");
    }
}
