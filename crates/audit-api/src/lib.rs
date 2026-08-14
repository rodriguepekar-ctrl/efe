// SPDX-License-Identifier: UNLICENSED
//! # `audit-api` — Provenance P1 / S2 : le service de LECTURE (« servir »)
//!
//! Le **pivot** entre le substrat scellé (S0 [`provenance_dag`] + S1 [`provenance_capture`]) et les
//! surfaces (console S3 ∥ export+verify S4 ∥ gabarits S6). Tout le **trust-critique** vit ici, en Rust ;
//! la console n'est qu'une **vue**. Quatre devoirs, chacun dans son module :
//!
//! - **projeter** ([`project`]) — `records → DecisionView`, une **fonction pure** qui n'invente rien
//!   (chaque champ nomme son `backed_by_node`) et ne peut **diverger** du record.
//! - **re-prouver à la lecture** ([`monitor`]) — badge rapide (cache clé-é par le manifest-root, sain
//!   car le root **engage** les records) + re-preuve **froide autoritaire** ; échec **FORT**, jamais masqué.
//! - **enveloppe honnête** ([`envelope`]) — « intègre ≠ correct » comme **TYPE** ; aucun champ « correct ».
//! - **isoler** ([`registry`], [`auth`]) — un store/clé/contexte par tenant ; le `{t}` du chemin doit
//!   égaler le tenant authentifié ; sceau clé-tenant en défense de profondeur. Plus l'unique **gate
//!   d'écriture** : l'action humaine scellée ([`service::AuditService::record_human_action`], §10).
//!
//! Avec, autour : l'[`index`] (cache reconstructible + pagination chain-stable), le [`router`]
//! transport-agnostique (HTTP-as-pure-function, air-gap-testable), le [`report`] (le récit Markdown), et
//! le seam d'**export** (contenu pour S4). Dépendances minimales : S0, S1, et `serde`/`serde_json` — pas
//! de framework HTTP async (la confiance ne dépend d'aucune coquille).
//!
//! ```
//! use audit_api::{TenantRegistry, AuthContext, Role, ProjectionConfig, route, Request, Method};
//! use provenance_dag::TenantKey;
//! use provenance_capture::LlmCall;
//!
//! let mut reg = TenantRegistry::new();
//! reg.provision("acme", TenantKey::derive(b"root", "acme"), ProjectionConfig::default());
//! // la capture S1 alimente le store du tenant
//! let auth = AuthContext::new("acme", Role::ComplianceOfficer, "officer:hanna");
//! {
//!     let svc = reg.resolve(&auth).unwrap();
//!     svc.ingest_llm(&LlmCall { case_id: "SUP-A", step: "sanction", agent_role: "sanction_check",
//!         inputs: b"acme", model: "phi4", version: "1", output: "PASS", evidence_steps: &[], timestamp: 1 }).unwrap();
//! }
//! // servir une décision : la projection + l'enveloppe d'intégrité, via le routeur pur
//! let resp = route(&mut reg, &Request { method: Method::Get, path: "/tenants/acme/cases/SUP-A".into(),
//!     query: vec![], auth, body: None, now: 100 });
//! assert_eq!(resp.status, 200);
//! assert!(resp.body.contains("INTEGRITY_ONLY"));
//! assert!(!resp.body.to_lowercase().contains("correct")); // « intègre ≠ correct », garanti par le type
//! ```

pub mod auth;
pub mod envelope;
pub mod index;
pub mod monitor;
pub mod project;
pub mod registry;
pub mod report;
pub mod router;
pub mod service;

pub use auth::{AuthContext, Role, TenantId};
pub use envelope::{IntegrityEnvelope, IntegritySemantics, IntegrityVerdict};
pub use index::{CaseEntry, CaseIndex};
pub use monitor::IntegrityMonitor;
pub use project::{
    derive_status, project, Check, DecisionStatus, DecisionView, EvidenceCitation, FlowEdge, OversightAction, ProjectionConfig,
};
pub use registry::{RegistryError, TenantRegistry};
pub use report::{render_case, render_export};
pub use router::{route, Method, Request, Response};
pub use service::{
    ApiError, AuditService, CaseResponse, CaseSummary, Cursor, ExportContent, HumanActionInput, HumanActionResp, ListQuery,
    ListResponse, SEALED_SCOPE_FULL,
};

// La **garde de souveraineté** héritée de S1 (§7), re-exportée pour que la surface publique de S2 porte
// visiblement le contrôle d'egress qu'elle « hérite » — la coquille de transport l'applique à son endpoint
// de bind (S2 lui-même n'a pas de socket sortant : air-gap par construction).
pub use provenance_capture::{assert_local, is_local_endpoint, SovereigntyError};
