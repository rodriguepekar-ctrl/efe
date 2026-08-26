// SPDX-License-Identifier: UNLICENSED
//! # `provenance-dag` — Provenance P1 / S0 : le DAG scellé re-prouvable (le **cœur de confiance**)
//!
//! Le contrat que tout P1 consomme : un graphe d'actes d'agents et de messages, **scellé** et
//! **re-prouvable indépendamment**, qui fait de « beaucoup de petits agents auditables » une réalité
//! cryptographique plutôt qu'une promesse. Cf. l'architecture P1 (`docs/reports/EFE_P1_*`).
//!
//! ## Le principe : ZÉRO dépendance crypto (trustless)
//! Tout le trust-critique — [`sha256`] (FIPS 180-4), [`hmac`] (RFC 2104), la dérivation de [`key`], le
//! scellement et la [`reprove_dag`] — est **écrit à la main, sans aucune crate crypto externe**, et
//! **prouvé par les vecteurs NIST/RFC**. Un auditeur peut lire le tout en une heure, le **recompiler**,
//! et vérifier qu'il fait ce qu'il dit — **sans faire confiance** ni à l'éditeur ni à une dépendance.
//! C'est ce qui fonde le futur vérifieur forensic autonome (S4). `serde`/`serde_json` ne servent qu'au
//! stockage JSONL d'inspection ; le **sceau est calculé sur un encodage canonique binaire**, pas sur le JSON.
//!
//! ## La sémantique, énoncée avec précision (intègre ≠ correct)
//! La re-preuve atteste l'**INTÉGRITÉ** : « ce DAG est le compte authentique, non-altéré, de ce qui
//! s'est passé — quels agents, ce qu'ils ont reçu, produit, et passé à qui, et ce que l'humain a
//! décidé ». Elle n'atteste **NI** la **correction** de la décision (couche de mesure, séparée), **NI**
//! une **reproductibilité bit-à-bit** (les LLM sont non-déterministes). Ne jamais confondre les deux —
//! c'est une nécessité légale autant qu'une exigence d'honnêteté (P1 §8).
//!
//! ## Les invariants, écrits comme tests (la signature EFE)
//! - **Falsification du sceau** : toute mutation d'un champ, ou tout réordonnancement, casse la re-preuve
//!   au point touché (`dag::tests::tampering_*`, `reordering_*`).
//! - **Trustless** : la re-preuve ne dépend que des records bruts + la clé (`reprove_dag`).
//! - **Souveraineté** : la clé est dérivée par tenant ; le client la détient (`key`).
//! - **Déterminisme** : même contenu + même clé → même manifest-root, à l'octet.
//! - **Intégrité structurelle** : références causales valides, ids uniques (sinon rejet).
//! - **Crypto correcte** : vecteurs NIST (SHA-256) et RFC 4231 (HMAC).
//!
//! ## Usage de bout en bout
//! ```
//! use provenance_dag::{Node, HumanAction, SealedDag, TenantKey, reprove_dag, verify_against_root};
//!
//! // Le client détient sa clé (souveraineté).
//! let key = TenantKey::derive(b"root-secret-du-client", "acme-corp");
//!
//! // On scelle, au fil de l'eau, les actes des agents (ici un acte minimal).
//! let mut dag = SealedDag::new();
//! dag.append(&key, Node::AgentAct {
//!     node_id: "sanction".into(), agent_role: "sanction_check".into(),
//!     inputs_hash: "h-entrees".into(), model: "phi4-mini".into(), version: "1".into(),
//!     output: "PASS".into(), evidence_refs: vec![], timestamp: 1,
//! }).unwrap();
//!
//! // Le manifest-root est l'engagement public ; on le publie/conserve.
//! let root = dag.manifest_root();
//!
//! // N'importe qui, avec les records bruts + la clé, re-prouve l'INTÉGRITÉ — sans confiance.
//! let verdict = reprove_dag(dag.records(), &key);
//! assert!(verdict.ok);
//! assert!(verify_against_root(dag.records(), &key, &root).ok);
//! ```

pub mod anchor;
pub mod dag;
pub mod hmac;
pub mod key;
pub mod node;
pub mod sha256;
/// **M6** — le crypto-shredding : la conservation limitée sur une chaîne qui ne se coupe pas.
pub mod shred;

pub use anchor::{
    verify_against_anchor, AnchorError, AnchorIndex, AnchorRecord, ANCHOR_SCHEMA_V1, meta_anchor_of, META_ANCHOR_PREFIX};
pub use dag::{reprove_dag, verify_against_root, DagError, Record, SealedDag, Verdict, GENESIS};
pub use hmac::{ct_eq, hmac_sha256};
pub use key::TenantKey;
pub use node::{HumanAction, Node};
pub use sha256::sha256;
pub use shred::{est_enveloppe, ouvrir, sceller, CaseKey, EtatCas, Lecture, Trousseau};
