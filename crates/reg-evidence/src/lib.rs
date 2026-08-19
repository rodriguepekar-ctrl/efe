// SPDX-License-Identifier: UNLICENSED
//! # `reg-evidence` — Provenance P1 / S6 : le **moteur d'évidence réglementaire** (les gabarits)
//!
//! La dernière marche de valeur de P1 : transformer le DAG scellé re-prouvable (S0/S1), servi (S2) et
//! exporté sans confiance (S4) en l'**artefact précis** qu'une équipe conformité doit produire **de toute
//! façon** — registre de tenue de logs (EU AI Act art. 12), dossier de diligence fournisseur (CSDDD/LkSG),
//! registre de traitement (GDPR) — et le produire **mieux** que la main : automatiquement, traçablement,
//! et **re-prouvable par le destinataire lui-même**. C'est ce qui réalise la thèse « moteur d'évidence »
//! (architecture P1 §7) et le **ROI** qui déclenche la conversion (heures → minutes). Cf.
//! `docs/reports/EFE_Kovant_P1_S6_RegEvidence_Architecture_2026-06-29.html`.
//!
//! ## Le principe : TRUST-LIGHT (zéro confiance neuve)
//! S6 n'invente **aucune** crypto et n'ajoute **aucune** surface de confiance. Tout son risque est dans
//! l'**honnêteté du cadrage**, pas dans le calcul. L'architecture tient en trois piliers, chacun imposé
//! par le type **et** par le test (la signature EFE : les invariants sont des tests qui cassent le build) :
//!
//! 1. **Une VUE re-dérivable** ([`derive`]) — `artefact = derive(records_scellés, mapping@version)` est une
//!    fonction **pure et déterministe**. Un tiers la ré-exécute depuis le bundle et obtient les **mêmes
//!    octets** → aucune confiance neuve (`I-S6-VUE-REDÉRIVABLE`). S6 **re-prouve lui-même** ([`reprove`])
//!    et **refuse** le non-prouvé (`I-S6-SUR-PROUVÉ`).
//! 2. **Un mapping versionné** ([`MappingSet`]) — « le nœud humain évidencie l'art. 14 » est une
//!    **revendication déclarée**, datée, **hachée** ([`MappingSet::set_hash`]), qui **voyage** avec
//!    l'artefact (`I-S6-MAPPING-VERSIONNÉ`). Le réglementaire évolue ; l'artefact dit **sous quelle
//!    lecture** il fut produit.
//! 3. **Une couverture honnête** ([`Coverage`]) — l'artefact **énumère ses trous** : un cadre sans
//!    [`Coverage::OutOfDagScope`] est **rejeté** ([`MappingError::NoDeclaredHoles`]). Taire un trou serait
//!    un « vert malhonnête » (la classe de bug attrapée en S2). C'est `I-S6-COUVERTURE-HONNÊTE`.
//!
//! ## L'honnêteté légalement nécessaire : OUTILLÉ ≠ CONFORME
//! Sœur de « intègre ≠ correct » (S2) et « intègre ≠ inforgeable » (S4). **Fournir et mapper** l'évidence
//! n'est **pas prononcer** la conformité (une conclusion juridique : classification du système, conseil du
//! client, obligations organisationnelles que le DAG ne produit pas). C'est imposé **par le type** : le
//! [`RegulatoryArtifact`] n'a **aucun champ** « conforme » (comme l'`IntegrityEnvelope` de S2 n'a aucun
//! champ « correct »). Et `intègre ≠ correct` est **propagé** : une décision scellée-mais-fausse n'est
//! **jamais** blanchie en « évidence de conformité » (l'évidence ne cite que des `node_id`, pas des verdicts).
//!
//! ## Dépendances
//! Le seul `provenance-dag` (S0) + `serde` — **aucune crypto neuve** (la dérivation re-prouve via S0).
//! `audit-api`/`provenance-capture` sont en dev-deps : l'export S2 alimente la dérivation, et le test
//! d'intégration **re-dérive comme un tiers** (trust-light, octet-à-octet).
//!
//! ## Usage de bout en bout
//! ```
//! use reg_evidence::{derive, render_md, templates, CaseScope, Framework};
//! use provenance_dag::{Node, SealedDag, TenantKey};
//!
//! // Le client détient sa clé (souveraineté). On scelle un acte d'agent minimal pour un cas.
//! let key = TenantKey::from_bytes([7u8; 32]);
//! let mut dag = SealedDag::new();
//! dag.append(&key, Node::AgentAct {
//!     node_id: "SUP-2024::sanction".into(), agent_role: "sanction_check".into(),
//!     inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(),
//!     output: "PASS".into(), evidence_refs: vec![], timestamp: 1,
//! }).unwrap();
//!
//! // On DÉRIVE l'artefact d'évidence EU AI Act pour ce dossier, sous la lecture (mapping) versionnée.
//! let mapping = templates::eu_ai_act_2026_06();
//! let artefact = derive(dag.records(), &key, &mapping, Framework::EuAiAct,
//!                       &CaseScope::Case("SUP-2024".into())).unwrap();
//!
//! // L'artefact est une VUE re-dérivable : un tiers re-fait derive(...) → le MÊME artefact.
//! assert_eq!(artefact, derive(dag.records(), &key, &mapping, Framework::EuAiAct,
//!                             &CaseScope::Case("SUP-2024".into())).unwrap());
//!
//! // Il enregistre la lecture (datable) et le root re-dérivé (lié au DAG).
//! assert_eq!(artefact.mapping_version, "2026-06");
//! assert_eq!(artefact.manifest_root, dag.manifest_root());
//!
//! // Le récit porte l'honnêteté, et n'affirme JAMAIS la conformité.
//! let md = render_md(&artefact);
//! assert!(md.to_lowercase().contains("outillé ≠ conforme"));
//! assert!(!md.to_lowercase().contains("compliant"));
//! ```

pub mod artifact;
pub mod bundle;
pub mod engine;
pub mod framework;
pub mod mapping;
pub mod render;
pub mod selector;
pub mod templates;

pub use artifact::{CaseScope, CoverageSummary, EvidenceSemantics, HonestyEnvelope, RegulatoryArtifact, RequirementFinding, SEMANTICS_NOTE};
pub use bundle::{reg_subtree, write_reg_subtree};
pub use engine::{derive, RegError};
pub use framework::{Coverage, Framework};
pub use mapping::{MappingError, MappingSet, RequirementMapping};
pub use render::{render_html, render_md};
pub use selector::{select_ids, Field, NodeKind, Selector};

// Le cœur de confiance réutilisé (S0), re-exporté pour que la surface de S6 porte visiblement la
// re-preuve dont elle dépend (aucune crypto neuve).
pub use provenance_dag::{reprove_dag as reprove, HumanAction, Node, Record, SealedDag, TenantKey, Verdict};
