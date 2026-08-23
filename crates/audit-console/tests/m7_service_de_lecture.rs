// SPDX-License-Identifier: UNLICENSED
//! # M7 — le service de lecture : le critère, exécuté.
//!
//! ## Le critère de la feuille de route était déjà satisfait
//!
//! *« `audit-api` répond en HTTP sur un port (aujourd'hui : bibliothèque) »*, prouvé par
//! `ls crates/audit-api/src/bin` → absent. Or la propriété **était vraie** : ce crate sert les six
//! routes de `audit_api::router::route` sur `/api/tenants/{t}/…`. La preuve mesurait
//! **l'emplacement d'un binaire**, pas la propriété. La feuille de route l'autorise elle-même :
//! *« si l'un de ces critères ne peut pas être exécuté, c'est le critère qu'il faut réécrire »*.
//!
//! ## Le critère réécrit, et ce qu'il débloque
//!
//! > **Un seul processus sert plusieurs tenants, et aucun ne voit l'autre.**
//!
//! `route()` refuse en 403 un `{t}` qui n'est pas celui du porteur, « sans même révéler l'existence
//! d'autres tenants ». Cette machinerie — la promesse de M8 — n'avait **aucun moyen d'être exercée
//! dans un service vivant**, parce que le binaire ne provisionnait qu'un tenant. C'est pour cela
//! que la feuille de route écrit `M7 → M8`, et c'est ce que ce fichier démontre.
//!
//! Le second devoir est la **parité** : l'invariant `I-S3-VUE-PURE` affirme que `/api` rend
//! *exactement* ce que `route()` rendrait. Il n'était prouvé que sur **une route sur six**.

use std::collections::BTreeMap;

use audit_api::{route, AuthContext, Method, Request, Role, TenantRegistry};
use provenance_capture::LlmCall;
use audit_console::handler::handle;
use audit_console::http::HttpRequest;
use audit_console::provisioning::{appliquer, Deploiement};
use audit_console::Console;
use provenance_dag::TenantKey;

const RACINE: &[u8] = b"racine-de-test-m7";
const MAINTENANT: u64 = 1000;

/// La déclaration d'un service à deux tenants — la posture que M7 rend possible.
const DEUX_TENANTS: &str = r#"{
    "tenants": [{"id": "acme"}, {"id": "globex"}],
    "sessions": [
        {"token": "jeton-acme",   "tenant": "acme",   "role": "compliance_officer", "subject": "officer:hanna"},
        {"token": "jeton-globex", "tenant": "globex", "role": "auditor",            "subject": "auditor:ext"}
    ]
}"#;

fn get(path: &str, token: Option<&str>) -> HttpRequest {
    let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    HttpRequest::parse(format!("GET {path} HTTP/1.1\r\n{auth}\r\n").as_bytes()).expect("requête")
}

fn post(path: &str, token: &str, body: &str) -> HttpRequest {
    let n = body.len();
    HttpRequest::parse(
        format!("POST {path} HTTP/1.1\r\nAuthorization: Bearer {token}\r\nContent-Length: {n}\r\n\r\n{body}")
            .as_bytes(),
    )
    .expect("requête")
}

/// Alimente un tenant avec un cas qui lui est propre — pour qu'« il voit ses données » soit
/// distinguable de « il voit quelque chose ».
fn semer(console: &mut Console, tenant: &str, cas: &str, verdict: &str) {
    let auth = AuthContext::new(tenant, Role::ComplianceOfficer, "seed");
    let svc = console.resolve_service(&auth).expect("tenant provisionné");
    svc.ingest_llm(&LlmCall {
        case_id: cas,
        step: "sanction",
        agent_role: "sanction_check",
        inputs: b"i",
        model: "phi4",
        version: "1",
        output: "PASS",
        evidence_steps: &[],
        timestamp: 1,
    })
    .expect("acte");
    svc.ingest_llm(&LlmCall {
        case_id: cas,
        step: "aggregate",
        agent_role: "verdict_aggregate",
        inputs: b"i",
        model: "phi4",
        version: "1",
        output: verdict,
        evidence_steps: &[],
        timestamp: 2,
    })
    .expect("verdict");
}

fn service_a_deux_tenants() -> Console {
    let mut c = Console::new();
    let dep = Deploiement::depuis_json(DEUX_TENANTS).expect("déclaration lisible");
    appliquer(&dep, &mut c, RACINE).expect("déclaration saine");
    semer(&mut c, "acme", "SUP-ACME", "COMPLIANT");
    semer(&mut c, "globex", "SUP-GLOBEX", "FAIL");
    c
}

// ═══ 1. LE CRITÈRE ════════════════════════════════════════════════════════════════════════════

/// **Le critère de M7.** Un processus, deux tenants, et chacun ne voit que le sien.
#[test]
fn m7_un_seul_service_sert_deux_tenants_et_aucun_ne_voit_lautre() {
    let mut c = service_a_deux_tenants();

    // Chacun lit CHEZ LUI — et lit bien SES données, pas « quelque chose ».
    let a = handle(&mut c, &get("/api/tenants/acme/cases/SUP-ACME", Some("jeton-acme")), MAINTENANT);
    assert_eq!(a.status, 200, "acme lit son cas");
    let corps_a = String::from_utf8_lossy(&a.body).to_string();
    assert!(corps_a.contains("SUP-ACME"), "et c'est bien le sien");

    let g = handle(&mut c, &get("/api/tenants/globex/cases/SUP-GLOBEX", Some("jeton-globex")), MAINTENANT);
    assert_eq!(g.status, 200, "globex lit le sien");
    assert!(String::from_utf8_lossy(&g.body).contains("SUP-GLOBEX"));

    // ── L'ISOLATION : acme vise globex. 403, et rien ne fuit. ──
    let croise = handle(&mut c, &get("/api/tenants/globex/cases/SUP-GLOBEX", Some("jeton-acme")), MAINTENANT);
    assert_eq!(croise.status, 403, "le jeton d'acme ne franchit pas la frontière de globex");
    let fuite = String::from_utf8_lossy(&croise.body).to_string();
    assert!(!fuite.contains("SUP-GLOBEX"), "aucune donnée : {fuite}");
    assert!(!fuite.contains("FAIL"), "pas même un verdict : {fuite}");

    // Et le refus est le MÊME pour un tenant qui n'existe pas — sinon comparer les deux réponses
    // révélerait quels tenants ce déploiement héberge.
    let inexistant = handle(&mut c, &get("/api/tenants/inexistant/cases/X", Some("jeton-acme")), MAINTENANT);
    assert_eq!(
        (inexistant.status, String::from_utf8_lossy(&inexistant.body).to_string()),
        (croise.status, fuite),
        "« tenant étranger » et « tenant inexistant » doivent être INDISTINGUABLES : les \
         distinguer transformerait la route en oracle d'existence de tenants"
    );
}

/// Le jeton partagé entre deux tenants : refusé **au démarrage**, et rien n'est monté.
///
/// Sans ce refus, `Sessions::insert` (un `HashMap::insert`) garderait la dernière session **en
/// silence**, et le porteur atteindrait l'autre tenant — l'isolation tomberait sur une faute de
/// frappe, dans un service qui aurait l'air de parfaitement fonctionner.
#[test]
fn m7_un_jeton_partage_arrete_le_demarrage_avant_douvrir_le_port() {
    let mut c = Console::new();
    let dep = Deploiement::depuis_json(
        r#"{
        "tenants": [{"id": "acme"}, {"id": "globex"}],
        "sessions": [
            {"token": "meme", "tenant": "acme",   "role": "auditor", "subject": "a"},
            {"token": "meme", "tenant": "globex", "role": "auditor", "subject": "b"}
        ]
    }"#,
    )
    .expect("lisible");

    let refus = appliquer(&dep, &mut c, RACINE).expect_err("doit refuser");
    assert!(refus.iter().any(|r| r.to_string().contains("JETON PARTAGÉ ENTRE DEUX TENANTS")));
    assert!(c.sessions().is_empty(), "aucune session ouverte");
    assert!(!c.est_provisionne(), "et le service n'est pas prêt : le port ne doit pas s'ouvrir");
}

/// Les rôles déclarés sont **appliqués**, pas décoratifs : l'auditeur de globex lit et exporte,
/// mais n'agit pas (« il atteste, il ne décide pas »).
#[test]
fn m7_les_roles_declares_sont_appliques() {
    let mut c = service_a_deux_tenants();
    let corps = r#"{"target_step":"aggregate","action":"APPROVE","justification":"revue"}"#;

    let officier = handle(
        &mut c,
        &post("/api/tenants/acme/cases/SUP-ACME/human-action", "jeton-acme", corps),
        MAINTENANT,
    );
    assert_eq!(officier.status, 200, "l'agent de conformité agit");

    let auditeur = handle(
        &mut c,
        &post("/api/tenants/globex/cases/SUP-GLOBEX/human-action", "jeton-globex", corps),
        MAINTENANT,
    );
    assert_eq!(auditeur.status, 403, "l'auditeur atteste, il ne décide pas");
}

// ═══ 2. LA PARITÉ — I-S3-VUE-PURE sur les SIX routes ══════════════════════════════════════════

/// Les six routes de `route()`, chacune avec une requête qui l'atteint.
/// `(nom, méthode, chemin S2, corps)` — le chemin de la coquille est `/api` + ce chemin.
const PARITE: &[(&str, &str, &str, Option<&str>)] = &[
    ("cases", "GET", "/tenants/acme/cases", None),
    ("cases/{id}", "GET", "/tenants/acme/cases/SUP-ACME", None),
    ("cases/{id}/report", "GET", "/tenants/acme/cases/SUP-ACME/report", None),
    (
        "cases/{id}/human-action",
        "POST",
        "/tenants/acme/cases/SUP-ACME/human-action",
        Some(r#"{"target_step":"aggregate","action":"APPROVE","justification":"revue"}"#),
    ),
    ("reprove", "POST", "/tenants/acme/reprove", None),
    ("export", "GET", "/tenants/acme/export", None),
];

/// Un registre S2 nu, dans **exactement** l'état de la console — pour comparer ce que `route()`
/// rendrait à ce que la coquille rend.
fn registre_jumeau() -> TenantRegistry {
    let mut reg = TenantRegistry::new();
    reg.provision("acme", TenantKey::derive(RACINE, "acme"), audit_api::ProjectionConfig::default());
    reg.provision("globex", TenantKey::derive(RACINE, "globex"), audit_api::ProjectionConfig::default());
    for (t, cas, verdict) in [("acme", "SUP-ACME", "COMPLIANT"), ("globex", "SUP-GLOBEX", "FAIL")] {
        let auth = AuthContext::new(t, Role::ComplianceOfficer, "seed");
        let svc = reg.resolve(&auth).expect("provisionné");
        for (step, role, out, ts) in
            [("sanction", "sanction_check", "PASS", 1u64), ("aggregate", "verdict_aggregate", verdict, 2)]
        {
            svc.ingest_llm(&LlmCall {
                case_id: cas,
                step,
                agent_role: role,
                inputs: b"i",
                model: "phi4",
                version: "1",
                output: out,
                evidence_steps: &[],
                timestamp: ts,
            })
            .expect("acte");
        }
    }
    reg
}

/// **`I-S3-VUE-PURE`, sur les six routes.** La coquille n'ajoute aucune décision : même statut,
/// même corps, pour chacune. Il n'en était prouvé qu'une.
#[test]
fn m7_la_coquille_rend_exactement_ce_que_route_rendrait_sur_les_six_routes() {
    for (nom, methode, chemin, corps) in PARITE {
        // Deux états IDENTIQUES : l'un derrière la coquille, l'autre nu. Les routes POST mutent —
        // comparer sur un état partagé ne prouverait rien.
        let mut console = service_a_deux_tenants();
        let mut jumeau = registre_jumeau();

        let via_coquille = match (*methode, corps) {
            ("GET", _) => handle(&mut console, &get(&format!("/api{chemin}"), Some("jeton-acme")), MAINTENANT),
            ("POST", Some(b)) => handle(&mut console, &post(&format!("/api{chemin}"), "jeton-acme", b), MAINTENANT),
            ("POST", None) => handle(&mut console, &post(&format!("/api{chemin}"), "jeton-acme", ""), MAINTENANT),
            _ => unreachable!(),
        };

        let direct = route(
            &mut jumeau,
            &Request {
                method: if *methode == "GET" { Method::Get } else { Method::Post },
                path: (*chemin).to_string(),
                query: vec![],
                auth: AuthContext::new("acme", Role::ComplianceOfficer, "officer:hanna"),
                body: corps.map(|b| b.to_string()),
                now: MAINTENANT,
            },
        );

        assert_eq!(via_coquille.status, direct.status, "{nom} : même statut que route()");
        assert_eq!(
            String::from_utf8_lossy(&via_coquille.body),
            direct.body,
            "{nom} : même corps que route() — la coquille n'ajoute aucune décision"
        );
    }
}

/// **Le garde de la parité.** Une route ajoutée à `route()` sans cas de parité doit faire tomber ce
/// test — sinon l'invariant `I-S3-VUE-PURE` continuerait d'être *affirmé* pour toute la surface
/// `/api` tout en n'étant *prouvé* que sur ce qu'on a pensé à écrire. C'est exactement l'état dans
/// lequel M7 a trouvé le dépôt : un invariant sur six routes, testé sur une.
#[test]
fn m7_chaque_route_de_route_a_son_cas_de_parite() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../audit-api/src/router.rs"
    ))
    .expect("router.rs lisible depuis le crate voisin");
    // Hors du module de test du routeur : ses fixtures emploient la même écriture.
    let hors_tests = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
    let arms = hors_tests.matches("(Method::Get, [").count() + hors_tests.matches("(Method::Post, [").count();

    assert_eq!(
        PARITE.len(),
        arms,
        "route() a {arms} routes et la table de parité en couvre {} : toute route ajoutée doit \
         venir avec la preuve qu'elle traverse la coquille sans être modifiée",
        PARITE.len()
    );

    // Et les cas doivent être DISTINCTS — six entrées identiques satisferaient le compte.
    let mut vus: BTreeMap<&str, ()> = BTreeMap::new();
    for (nom, _, chemin, _) in PARITE {
        assert!(vus.insert(chemin, ()).is_none(), "cas de parité dupliqué : {nom}");
    }
}

// ═══ 3. LES GARDES ════════════════════════════════════════════════════════════════════════════

/// La portion de CODE d'une fonction de `main.rs` — commentaires retirés.
///
/// Le retrait n'est pas cosmétique : la prose de `main.rs` **nomme** les interrupteurs qu'elle
/// explique, et un garde textuel qui lit les commentaires se trompe dans les deux sens — il peut
/// refuser du code correct, et se laisser satisfaire par un commentaire décrivant du code absent.
fn corps_de(src: &str, debut: &str) -> String {
    let i = src.find(debut).unwrap_or_else(|| panic!("`{debut}` introuvable dans main.rs"));
    let reste = &src[i..];
    let fin = reste[1..].find("\n}\n").map_or(reste.len(), |j| j + 3);
    reste[..fin]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn main_rs() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs")).expect("main.rs")
}

/// **Le garde de `SUPPLANTES_PAR_DECLARATION`.** Cette liste décide de ce que le rapport M0 annonce
/// « SANS OBJET ». Si elle dérivait de ce que le binaire lit réellement, on obtiendrait l'un des
/// deux mensonges symétriques :
///
/// * un interrupteur annoncé sans objet alors qu'il agit — l'opérateur le retire, et casse ;
/// * un interrupteur annoncé dégradé alors qu'il est ignoré — la fausse alarme de chaque démarrage.
#[test]
fn m7_la_liste_des_supplantes_est_exactement_ce_que_le_binaire_lit() {
    let src = main_rs();
    let demo = corps_de(&src, "fn monter_demonstration");
    let declare = corps_de(&src, "fn monter_declare");

    for nom in audit_console::profile::SUPPLANTES_PAR_DECLARATION {
        assert!(
            demo.contains(nom),
            "{nom} est annoncé « supplanté par la déclaration », mais la posture de démonstration \
             ne le lit pas : la liste ne décrit plus le binaire"
        );
        assert!(
            !declare.contains(nom),
            "{nom} est annoncé « sans objet » en posture déclarée, et pourtant `monter_declare` le \
             lit : le rapport dirait à l'opérateur de retirer une variable qui agit"
        );
    }

    // Et la réciproque : rien d'autre que ces quatre-là ne doit être lu par la seule démonstration.
    // `EFE_CONSOLE_ROOT` et `EFE_CONSOLE_ADDR` sont lus par le `main` commun, pas ici.
    for autre in ["EFE_CONSOLE_ROOT", "EFE_CONSOLE_ADDR", "EFE_CONSOLE_DEPLOIEMENT"] {
        assert!(
            !demo.contains(autre),
            "{autre} n'est PAS supplanté — il ne doit pas être lu dans la seule posture de démonstration"
        );
    }
}

/// L'exemple livré doit **parser et valider**. Un exemple périmé est pire que pas d'exemple : il
/// est recopié, il échoue au démarrage, et c'est le format qu'on accuse.
#[test]
fn m7_lexemple_livre_est_valide() {
    let chemin = concat!(env!("CARGO_MANIFEST_DIR"), "/../../profiles/deploiement.exemple.json");
    let brut = std::fs::read_to_string(chemin).expect("profiles/deploiement.exemple.json lisible");
    let dep = Deploiement::depuis_json(&brut).expect("exemple bien formé");
    dep.valider().expect("exemple valide — sinon il enseigne une erreur");

    assert_eq!(dep.tenants.len(), 2, "l'exemple MONTRE le multi-tenant, c'est tout son objet");
    assert!(dep.sessions.len() >= 3, "et plusieurs rôles");
    let note = dep.note.as_deref().unwrap_or_default();
    assert!(note.contains("NE PAS DÉPLOYER"), "l'exemple dit qu'il est un exemple : {note}");

    // Deux rôles distincts au moins : un exemple mono-rôle n'enseignerait pas le vocabulaire.
    let roles: std::collections::BTreeSet<&str> = dep.sessions.iter().map(|s| s.role.as_str()).collect();
    assert!(roles.len() >= 3, "l'exemple montre plusieurs rôles : {roles:?}");
}
