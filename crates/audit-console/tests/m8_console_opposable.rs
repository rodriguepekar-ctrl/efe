// SPDX-License-Identifier: UNLICENSED
//! # M8 — la console opposable : le critère, exécuté.
//!
//! *« auth + TLS + un tenant ne voit **jamais** l'autre. »* M7 a rendu la troisième clause testable
//! en faisant tenir plusieurs tenants à un seul processus. M8 traite les deux premières — et il faut
//! d'abord dire ce qu'elles voulaient dire ici.
//!
//! ## Ce que « opposable » exige
//!
//! Un acte de supervision est scellé dans une chaîne **non répudiable** : `human_id`, `action`,
//! `justification`. Il n'est **opposable** à quelqu'un que si l'on peut tenir les conditions dans
//! lesquelles il a été obtenu. Or `efe-nano-sdk` a déjà nommé exactement ce trou, pour la
//! computation :
//!
//! > *« Aujourd'hui la localité est invisible dans les octets scellés : un record produit à distance
//! > serait byte-identique à un local. »*
//!
//! Transposé : une approbation venue d'un officier en boucle locale, de quelqu'un qui a lu le jeton
//! dans un fichier, ou d'un site tiers qui a forgé la requête, produit des octets **identiques**.
//!
//! ## Ce que M8 fait, et pourquoi il COUPE au lieu d'enregistrer
//!
//! Enregistrer le barreau dans le nœud demanderait de changer le format de `Node::Human` — un
//! événement de version, pas un module. Alors on applique la règle de M3 : *dégrader produit-il
//! quelque chose d'utile **et honnête** ?* Lire sur un canal faible perd de la confidentialité ;
//! **sceller** sur un canal faible inscrit une attestation que rien ne soutient, irréversiblement.
//!
//! > **La lecture dégrade, l'écriture coupe.**

use audit_console::canal::{Canal, Plancher};
use audit_console::handler::{handle, PUBLIQUES};
use audit_console::http::{HttpRequest, HttpResponse};
use audit_console::provisioning::{appliquer, Deploiement};
use audit_console::Console;
use provenance_capture::LlmCall;

use audit_api::{AuthContext, Role};

const RACINE: &[u8] = b"racine-de-test-m8";
const T: u64 = 1000;
const LOCAL: Canal = Canal::A1LoopbackTemoin;
const NU: Canal = Canal::A3NonProtege;

fn declare() -> Canal {
    Canal::A2ProtegeDeclare { terminateur: "ingress-de-test".into(), corrobore: false }
}

const DEUX_TENANTS: &str = r#"{
    "tenants": [{"id": "acme"}, {"id": "globex"}],
    "sessions": [
        {"token": "jeton-acme",   "tenant": "acme",   "role": "compliance_officer", "subject": "officer:hanna"},
        {"token": "jeton-globex", "tenant": "globex", "role": "compliance_officer", "subject": "officer:otto"}
    ]
}"#;

/// Le corps d'un acte de supervision — celui qui SCELLE.
const ACTE: &str = r#"{"target_step":"aggregate","action":"APPROVE","justification":"revue"}"#;

fn service() -> Console {
    let mut c = Console::new();
    let d = Deploiement::depuis_json(DEUX_TENANTS).expect("lisible");
    appliquer(&d, &mut c, RACINE, 0).expect("saine");
    for (t, cas) in [("acme", "SUP-ACME"), ("globex", "SUP-GLOBEX")] {
        let auth = AuthContext::new(t, Role::ComplianceOfficer, "seed");
        let svc = c.resolve_service(&auth).expect("provisionné");
        for (step, out, ts) in [("sanction", "PASS", 1u64), ("aggregate", "COMPLIANT", 2)] {
            svc.ingest_llm(&LlmCall {
                case_id: cas,
                step,
                agent_role: "verdict_aggregate",
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
    c
}

fn get(path: &str, token: Option<&str>) -> HttpRequest {
    let a = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    HttpRequest::parse(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:8088\r\n{a}\r\n").as_bytes())
        .expect("requête")
}

/// POST authentifié par **en-tête** — le cas d'un client machine.
fn post_entete(path: &str, token: &str, body: &str) -> HttpRequest {
    HttpRequest::parse(
        format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:8088\r\nAuthorization: Bearer {token}\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    )
    .expect("requête")
}

/// POST authentifié par **cookie**, avec des en-têtes d'origine au choix — le cas du navigateur.
fn post_cookie(path: &str, token: &str, body: &str, origine: &str) -> HttpRequest {
    HttpRequest::parse(
        format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:8088\r\nCookie: session={token}\r\n{origine}\
             Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    )
    .expect("requête")
}

fn corps(r: &HttpResponse) -> String {
    String::from_utf8_lossy(&r.body).to_string()
}

// ═══ 1. LE PLANCHER : la lecture dégrade, l'écriture coupe ════════════════════════════════════

/// **Le critère de M8.** Sur un canal non protégé, on **lit** — et on ne **scelle** rien.
#[test]
fn m8_sur_un_canal_faible_on_lit_mais_on_ne_scelle_rien() {
    let mut c = service();

    // Lire : autorisé. Une lecture sur un canal faible perd de la confidentialité — c'est grave,
    // c'est réparable en changeant de canal, et cela n'inscrit rien nulle part.
    let lu = handle(&mut c, &get("/api/tenants/acme/cases/SUP-ACME", Some("jeton-acme")), T, &NU);
    assert_eq!(lu.status, 200, "lire reste possible sur un canal non protégé");

    // Sceller : refusé. Un acte scellé sur ce canal serait une attestation non répudiable que rien
    // ne soutient — et rien ne pourrait la retirer ensuite.
    let scelle = handle(
        &mut c,
        &post_entete("/api/tenants/acme/cases/SUP-ACME/human-action", "jeton-acme", ACTE),
        T,
        &NU,
    );
    assert_eq!(scelle.status, 403, "sceller est refusé sur un canal non protégé");
    let msg = corps(&scelle);
    assert!(msg.contains("Lire reste possible"), "le refus distingue lire d'écrire : {msg}");
    assert!(msg.contains("non r\\u00e9pudiable") || msg.contains("non répudiable"), "et dit pourquoi : {msg}");
    assert!(msg.contains("a3_non_protege"), "et nomme le barreau constaté : {msg}");
}

/// En boucle locale, tout passe — c'est **exactement** le comportement d'avant M8.
#[test]
fn m8_en_boucle_locale_le_scellement_passe_comme_avant() {
    let mut c = service();
    let r = handle(
        &mut c,
        &post_entete("/api/tenants/acme/cases/SUP-ACME/human-action", "jeton-acme", ACTE),
        T,
        &LOCAL,
    );
    assert_eq!(r.status, 200, "le défaut de M8 ne change rien au chemin servi d'avant");
}

/// Un terminateur **déclaré** abaisse le plancher à A2 — et pas plus bas.
#[test]
fn m8_un_terminateur_declare_autorise_le_scellement_a_distance_mais_pas_sans_lui() {
    let mut c = service();
    c.avec_plancher(Plancher::protege_declare());

    let via_declare = handle(
        &mut c,
        &post_entete("/api/tenants/acme/cases/SUP-ACME/human-action", "jeton-acme", ACTE),
        T,
        &declare(),
    );
    assert_eq!(via_declare.status, 200, "canal déclaré : le scellement est admis");

    let mut c2 = service();
    c2.avec_plancher(Plancher::protege_declare());
    let sans = handle(
        &mut c2,
        &post_entete("/api/tenants/acme/cases/SUP-ACME/human-action", "jeton-acme", ACTE),
        T,
        &NU,
    );
    assert_eq!(sans.status, 403, "abaisser le plancher à A2 n'ouvre JAMAIS A3");
}

/// Le plancher ne s'applique qu'aux routes **scellantes** : `/reprove` lit, il n'écrit rien.
#[test]
fn m8_le_plancher_ne_frappe_que_ce_qui_scelle() {
    let mut c = service();
    let r = handle(&mut c, &post_entete("/api/tenants/acme/reprove", "jeton-acme", ""), T, &NU);
    assert_ne!(r.status, 403, "re-prouver ne scelle rien : le plancher ne s'y applique pas");
}

// ═══ 2. LE BARREAU VIENT DE LA CONNEXION, JAMAIS D'UN EN-TÊTE ═════════════════════════════════

/// **La propriété qui rend l'échelle non décorative.** Aucun en-tête ne doit pouvoir hisser le
/// barreau : un client qui pourrait annoncer son propre niveau de confiance l'annulerait.
#[test]
fn m8_aucun_en_tete_ne_hisse_le_barreau() {
    const TENTATIVES: [&str; 6] = [
        "X-Forwarded-Proto: https\r\n",
        "X-Forwarded-For: 127.0.0.1\r\n",
        "Forwarded: for=127.0.0.1;proto=https\r\n",
        "X-Canal: a1_loopback_temoin\r\n",
        "Canal: a1_loopback_temoin\r\n",
        "X-Real-IP: 127.0.0.1\r\n",
    ];
    for t in TENTATIVES {
        let mut c = service();
        let req = HttpRequest::parse(
            format!(
                "POST /api/tenants/acme/cases/SUP-ACME/human-action HTTP/1.1\r\nHost: h\r\n\
                 Authorization: Bearer jeton-acme\r\n{t}Content-Length: {}\r\n\r\n{ACTE}",
                ACTE.len()
            )
            .as_bytes(),
        )
        .expect("requête");
        // Le canal passé reste A3 : c'est la CONNEXION qui parle, pas la requête.
        let r = handle(&mut c, &req, T, &NU);
        assert_eq!(r.status, 403, "« {} » ne doit rien hisser", t.trim());
    }
}

// ═══ 3. CSRF : un cookie part tout seul, un en-tête non ═══════════════════════════════════════

/// **Le trou que M8 ferme.** Un site tiers pouvait faire sceller une approbation sous l'identité
/// d'un officier — et le record aurait été indistinguable d'un vrai.
#[test]
fn m8_une_requete_inter_site_authentifiee_par_cookie_est_refusee() {
    let mut c = service();
    let r = handle(
        &mut c,
        &post_cookie("/cases/SUP-ACME/action", "jeton-acme",
                     "target_step=aggregate&action=APPROVE&justification=forge",
                     "Sec-Fetch-Site: cross-site\r\n"),
        T,
        &LOCAL,
    );
    assert_eq!(r.status, 403, "une requête inter-site ne scelle rien");
    assert!(corps(&r).contains("inter-site"), "et le dit : {}", corps(&r));
}

/// Sans **aucune** preuve d'origine, un POST par cookie est refusé : un navigateur ancien ou un
/// client forgé ne doit pas passer par défaut.
#[test]
fn m8_un_post_par_cookie_sans_preuve_dorigine_est_refuse() {
    let mut c = service();
    let r = handle(
        &mut c,
        &post_cookie("/cases/SUP-ACME/action", "jeton-acme", "target_step=aggregate&action=APPROVE&justification=x", ""),
        T,
        &LOCAL,
    );
    assert_eq!(r.status, 403);
}

/// Avec une preuve d'origine — `Sec-Fetch-Site` ou `Origin` concordant — il passe.
#[test]
fn m8_un_post_par_cookie_avec_preuve_dorigine_passe() {
    for preuve in ["Sec-Fetch-Site: same-origin\r\n", "Origin: http://127.0.0.1:8088\r\n"] {
        let mut c = service();
        let r = handle(
            &mut c,
            &post_cookie("/cases/SUP-ACME/action", "jeton-acme",
                         "target_step=aggregate&action=APPROVE&justification=revue", preuve),
            T,
            &LOCAL,
        );
        assert_ne!(r.status, 403, "« {} » est une preuve d'origine valide", preuve.trim());
    }
}

/// Un `Origin` **discordant** est refusé — et la comparaison passe par l'analyseur d'autorité
/// unique de M1, pas par un `split` local.
#[test]
fn m8_un_origin_discordant_est_refuse() {
    let mut c = service();
    let r = handle(
        &mut c,
        &post_cookie("/cases/SUP-ACME/action", "jeton-acme", "target_step=aggregate&action=APPROVE&justification=x",
                     "Origin: http://mechant.example\r\n"),
        T,
        &LOCAL,
    );
    assert_eq!(r.status, 403);
}

/// **L'en-tête porteur est dispensé, et c'est un raisonnement, pas une commodité** : un site tiers
/// ne peut pas faire poser `Authorization` par un navigateur sur une requête simple.
#[test]
fn m8_len_tete_porteur_est_dispense() {
    let mut c = service();
    let r = handle(
        &mut c,
        &post_entete("/api/tenants/acme/cases/SUP-ACME/human-action", "jeton-acme", ACTE),
        T,
        &LOCAL,
    );
    assert_eq!(r.status, 200, "un client machine n'a aucun en-tête d'origine à fournir");
}

// ═══ 4. LA SESSION : connexion, cookie, échéance ══════════════════════════════════════════════

/// Le cookie posé porte `HttpOnly` et `SameSite=Strict`, et `Secure` **seulement** hors boucle
/// locale — le poser sur `http://127.0.0.1` empêcherait le navigateur d'enregistrer le cookie,
/// donc casserait la console locale.
#[test]
fn m8_le_cookie_de_session_est_scelle_selon_le_canal() {
    let form = "token=jeton-acme";
    let req = |o: &str| {
        HttpRequest::parse(
            format!("POST /login HTTP/1.1\r\nHost: 127.0.0.1:8088\r\n{o}Content-Length: {}\r\n\r\n{form}", form.len())
                .as_bytes(),
        )
        .expect("requête")
    };

    let mut c = service();
    let local = handle(&mut c, &req("Sec-Fetch-Site: same-origin\r\n"), T, &LOCAL);
    let sc = local.headers.iter().find(|(k, _)| k == "Set-Cookie").expect("cookie posé").1.clone();
    assert_eq!(local.status, 303);
    assert!(sc.contains("HttpOnly"), "aucun script ne lit la session : {sc}");
    assert!(sc.contains("SameSite=Strict"), "aucun site tiers ne la fait partir : {sc}");
    assert!(!sc.contains("Secure"), "sur http://127.0.0.1, `Secure` empêcherait l'enregistrement : {sc}");

    let mut c2 = service();
    let distant = handle(&mut c2, &req("Sec-Fetch-Site: same-origin\r\n"), T, &declare());
    let sc2 = distant.headers.iter().find(|(k, _)| k == "Set-Cookie").expect("cookie").1.clone();
    assert!(sc2.contains("Secure"), "hors boucle locale, le cookie exige un canal chiffré : {sc2}");
}

/// Une connexion **forcée depuis un autre site** est refusée : sans cela, l'officier travaillerait
/// dans la session de l'attaquant, et les actes qu'il scellerait iraient chez lui.
#[test]
fn m8_une_connexion_inter_site_est_refusee() {
    let form = "token=jeton-acme";
    let mut c = service();
    let req = HttpRequest::parse(
        format!("POST /login HTTP/1.1\r\nHost: h\r\nSec-Fetch-Site: cross-site\r\nContent-Length: {}\r\n\r\n{form}", form.len())
            .as_bytes(),
    )
    .expect("requête");
    let r = handle(&mut c, &req, T, &LOCAL);
    assert_eq!(r.status, 403);
    assert!(r.headers.iter().all(|(k, _)| k != "Set-Cookie"), "et aucune session n'est ouverte");
}

/// Un jeton refusé ne dit pas **pourquoi** : distinguer « inconnu » d'« échu » apprendrait à un
/// porteur non authentifié que ce jeton a existé.
#[test]
fn m8_une_connexion_refusee_ne_dit_pas_pourquoi() {
    let mut c = service();
    let f = |t: &str| {
        let b = format!("token={t}");
        HttpRequest::parse(
            format!("POST /login HTTP/1.1\r\nHost: h\r\nSec-Fetch-Site: same-origin\r\nContent-Length: {}\r\n\r\n{b}", b.len())
                .as_bytes(),
        )
        .expect("requête")
    };
    let a = handle(&mut c, &f("jamais-vu"), T, &LOCAL);
    let b = handle(&mut c, &f("autre-inconnu"), T, &LOCAL);
    assert_eq!((a.status, corps(&a)), (b.status, corps(&b)), "réponses identiques");
    assert!(a.headers.iter().all(|(k, _)| k != "Set-Cookie"));
}

// ═══ 5. L'ISOLATION TIENT SOUS TOUS LES CANAUX ════════════════════════════════════════════════

/// La clause de M7 ne doit pas se relâcher parce que M8 a ouvert le service à distance.
#[test]
fn m8_lisolation_tient_sous_chaque_barreau() {
    for canal in [LOCAL, declare(), NU, Canal::NonAssertee] {
        let mut c = service();
        let r = handle(&mut c, &get("/api/tenants/globex/cases/SUP-GLOBEX", Some("jeton-acme")), T, &canal);
        assert_eq!(r.status, 403, "sous {canal}, acme ne franchit pas la frontière de globex");
        assert!(!corps(&r).contains("SUP-GLOBEX"), "et rien ne fuit");
    }
}

/// Les routes publiques restent publiques **sous tous les barreaux** — une sonde ne doit pas
/// dépendre du canal, sinon un superviseur distant ne pourrait plus surveiller.
#[test]
fn m8_les_routes_publiques_ne_dependent_pas_du_canal() {
    for canal in [LOCAL, declare(), NU] {
        let mut c = service();
        for (m, p) in PUBLIQUES.iter().filter(|(m, _)| *m == "GET") {
            let r = handle(&mut c, &get(p, None), T, &canal);
            assert_ne!(r.status, 401, "{m} {p} sous {canal}");
            assert_ne!(r.status, 403, "{m} {p} sous {canal}");
        }
    }
}

// ═══ 5. LA CORROBORATION : du pair TCP jusqu'au refus d'écriture ═══════════════════════════════
//
// M8 déclarait, en toutes lettres, une limite qu'il tenait pour définitive :
//
// > *« La déclaration vaut pour le PROCESSUS, pas pour la connexion : un client qui contourne le
// > terminateur et se connecte directement est crédité "protégé". Le seul témoin serait un en-tête,
// > et un en-tête est falsifiable par le client — donc inutilisable ici. »*
//
// La première phrase était juste. La seconde était **fausse**, et c'est elle qui fermait la porte :
// il existe un témoin qui n'est pas un en-tête. **L'adresse source d'une connexion TCP n'est pas
// envoyée par le client — elle est remplie par le noyau à la poignée de main.** Un client peut
// mentir sur `X-Forwarded-For` ; il ne peut pas mentir sur l'adresse depuis laquelle il a ouvert
// la connexion, sauf à ne jamais recevoir la réponse.
//
// L'exploitant déclare donc *d'où* son terminateur se connecte (`EFE_CONSOLE_TERMINATEUR_ORIGINE`),
// et le contournement cesse d'être crédité. Ces trois tests le prouvent **de bout en bout** : d'un
// `SocketAddr` de pair jusqu'au code HTTP.

/// Le pair correspond à une origine déclarée : le scellement passe, et il est CORROBORÉ.
#[test]
fn m8_le_terminateur_corrobore_scelle() {
    let origines = ["10.0.0.5".parse().expect("ip")];
    let canal = Canal::observer(Some("10.0.0.5:41000".parse().expect("pair")), Some("ingress"), &origines);
    assert!(matches!(canal, Canal::A2ProtegeDeclare { corrobore: true, .. }), "{canal}");

    let mut c = service();
    c.avec_plancher(Plancher::protege_declare());
    let r = handle(
        &mut c,
        &post_entete("/api/tenants/acme/cases/SUP-ACME/human-action", "jeton-acme", ACTE),
        T,
        &canal,
    );
    assert_eq!(r.status, 200, "une connexion venue du terminateur déclaré scelle");
}

/// **Le contournement.** Même déclaration, même plancher — mais la connexion ne vient pas de là.
/// Elle lit toujours ; elle ne scelle plus. C'est la limite de M8, close.
#[test]
fn m8_le_contournement_du_terminateur_lit_encore_mais_ne_scelle_plus() {
    let origines = ["10.0.0.5".parse().expect("ip")];
    // Un client qui a trouvé le port du service et court-circuite l'ingress.
    let canal = Canal::observer(Some("203.0.113.9:41000".parse().expect("pair")), Some("ingress"), &origines);
    assert_eq!(canal, Canal::A3NonProtege, "il n'est pas un A2 dégradé : il n'est pas un A2");

    let mut c = service();
    c.avec_plancher(Plancher::protege_declare());

    let lu = handle(&mut c, &get("/api/tenants/acme/cases/SUP-ACME", Some("jeton-acme")), T, &canal);
    assert_eq!(lu.status, 200, "la lecture dégrade — elle n'inscrit rien");

    let scelle = handle(
        &mut c,
        &post_entete("/api/tenants/acme/cases/SUP-ACME/human-action", "jeton-acme", ACTE),
        T,
        &canal,
    );
    assert_eq!(scelle.status, 403, "l'écriture coupe : rien de non répudiable ne s'inscrit ici");
    assert!(corps(&scelle).contains("a3_non_protege"), "et le refus nomme le barreau constaté");
}

/// **La compatibilité, prouvée et pas promise.** Sans origine déclarée, le comportement livré par
/// M8 est intact — y compris sa limite résiduelle, que seul l'exploitant peut lever.
#[test]
fn m8_sans_origine_declaree_le_comportement_livre_est_intact() {
    for pair in ["10.0.0.5:41000", "203.0.113.9:41000"] {
        let canal = Canal::observer(Some(pair.parse().expect("pair")), Some("ingress-de-test"), &[]);
        assert_eq!(canal, declare(), "sans origine, tout pair distant reste un A2 non corroboré");

        let mut c = service();
        c.avec_plancher(Plancher::protege_declare());
        let r = handle(
            &mut c,
            &post_entete("/api/tenants/acme/cases/SUP-ACME/human-action", "jeton-acme", ACTE),
            T,
            &canal,
        );
        assert_eq!(r.status, 200, "{pair} : le déploiement A2 existant continue de sceller");
    }
}
