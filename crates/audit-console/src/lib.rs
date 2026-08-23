// SPDX-License-Identifier: UNLICENSED
//! # `audit-console` — Provenance P1 / S3 : la console opérationnelle de conformité (la 2ᵉ surface)
//!
//! La surface quotidienne de l'agent de conformité non-cryptographe, et la **dernière** marche de P1.
//! C'est la **seconde des deux surfaces** (l'autre étant l'export forensic *trustless* de S4) : la console
//! **opérationnelle**, où l'on **revoit, comprend, agit, et exporte — seul, sans ingénieur**. Cf.
//! `docs/reports/EFE_Kovant_P1_S3_Console_Architecture_2026-06-29.html`.
//!
//! ## Le principe : la console REND la confiance, elle ne l'ÉTABLIT pas
//! La frontière de confiance s'arrête à **S2** ([`audit_api`]) : tout le trust-critique (projection,
//! intégrité-à-la-lecture, RBAC, rédaction, gate d'écriture, seam d'export) y vit, **sans framework**,
//! **air-gap-testable** comme une fonction pure. S3 n'est que **deux coquilles NON trust-critiques** : une
//! coquille de **transport** ([`http`] + [`handler`] + [`server`], HTTP fait main, egress-gardé) et une
//! **interface** ([`render`], HTML rendu serveur). Conséquence de sécurité énoncée franchement : une
//! console compromise est un **risque opérationnel** (tromper l'opérateur), **jamais** une forgerie de
//! provenance (pas de clé ; la re-preuve froide de S2 et la re-preuve du tiers l'attrapent).
//!
//! ## L'honnêteté cardinale : MONTRÉ vérifié ≠ PROUVÉ sans confiance
//! Sœur de « intègre ≠ correct » (S2), « intègre ≠ inforgeable » (S4), « outillé ≠ conforme » (S6) : le
//! badge **rend** le verdict de S2 (confiance **opérationnelle**, vous êtes dans le tenant) ; la preuve
//! **opposable** est l'**export** re-prouvable (confiance **forensic**, personne à croire). C'est pour ça
//! que l'export est à un clic ([`export`]). Le rendu **porte** cette honnêteté, montre la **fraîcheur**
//! (`as_of`), et passe **fail-loud au rouge** sur un store altéré — jamais un faux vert.
//!
//! ## Dépendances
//! S2 ([`audit_api`], le routeur pur), S4 (`forensic-export`, l'assemblage) et S6 (`reg-evidence`, l'évidence
//! réglementaire) pour l'export — plus `serde`. **Zéro dépendance externe** : HTTP/1.1 et USTAR faits main.
//! La **clé tenant** vit dans le backend ([`app::Console`]) et ne traverse **jamais** vers le navigateur.
//!
//! ## Usage de bout en bout
//! ```
//! use audit_console::{Console, handle, HttpRequest, Role};
//! use audit_console::canal::Canal;
//! use provenance_dag::TenantKey;
//!
//! // Le backend détient la clé (souveraineté) ; la console est dans la frontière du tenant.
//! let mut console = Console::new();
//! console.provision("acme", TenantKey::from_bytes([7u8; 32]));
//! console.add_session("tok", "acme", Role::ComplianceOfficer, "officer:hanna");
//!
//! // Une requête sans session est refusée (401) ; avec session, on REND la vue.
//! let req = HttpRequest::parse(b"GET / HTTP/1.1\r\nAuthorization: Bearer tok\r\n\r\n").unwrap();
//! // M8 — le barreau du canal vient de la CONNEXION, jamais de la requête : il est donc un
//! // paramètre à part. Ici, la boucle locale — le seul barreau qu'un processus puisse témoigner.
//! let resp = handle(&mut console, &req, 0, &Canal::A1LoopbackTemoin);
//! assert_eq!(resp.status, 200);
//! let html = String::from_utf8(resp.body).unwrap();
//! // Le rendu porte l'honnêteté cardinale, et n'affiche jamais « correct » comme verdict.
//! assert!(html.contains("Montré vérifié ≠ prouvé sans confiance"));
//! assert!(!html.to_lowercase().contains(">correct<"));
//! ```

// M0 — le profil servi : registre des interrupteurs, seul lecteur d'environnement, et rapport.
/// **M8** — le barreau de confiance du canal, et le plancher qu'un acte humain exige.
pub mod canal;
pub mod profile;
/// **M7** — la déclaration d'un déploiement, et les refus qui gardent l'isolation.
pub mod provisioning;

pub mod app;
pub mod export;
pub mod handler;
pub mod http;
pub mod render;
pub mod server;
pub mod session;
pub mod tar;

pub use app::Console;
pub use export::{build_bundle_tar, BundleTar, ExportFail};
pub use handler::handle;
pub use http::{HttpMethod, HttpRequest, HttpResponse};
pub use server::serve;
pub use session::{extract_token, Session, Sessions};

// Le rôle de S2, re-exporté pour configurer les sessions sans dépendre directement de l'audit-api côté appelant.
pub use audit_api::Role;
