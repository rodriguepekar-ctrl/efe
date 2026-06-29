// SPDX-License-Identifier: UNLICENSED
//! # `forensic-export` — Kovant P1 / S4a : l'assembleur du bundle trustless (le fossé)
//!
//! Transforme le **seam d'export** de S2 ([`BundleInput`]) en un **bundle forensic autonome** qu'un tiers
//! re-prouve **hors-ligne, sans nous faire confiance**, en **re-compilant** lui-même le vérifieur fourni.
//! La discipline (cf. l'architecture S4) :
//! - **re-prouver** les records (défense en profondeur — pas de confiance en un flag) et **refuser** un
//!   store cassé ([`assemble`] → [`ExportError::BrokenStore`]) ;
//! - **vendorer** la SOURCE du vérifieur ([`vendor`]) — `provenance-verify` + `provenance-dag`, embarquée
//!   par `include_str!` — pour que le tiers la **re-compile** (jamais de binaire à truster) ;
//! - écrire le **récit** (une vue), le **MANIFEST** de hachages ([`manifest`]), et le **README du
//!   sceptique** ([`readme`]) qui porte « intègre ≠ correct » ET « **intègre ≠ inforgeable** » (l'ancrage) ;
//! - la **clé** re-prouve mais n'est **JAMAIS** écrite (I-S4-SANS-CLÉ, structurel).
//!
//! Dépend du seul `provenance-dag` (cohérent avec le fossé : zéro dépendance crypto externe).
//!
//! ```
//! use forensic_export::{assemble, BundleInput, BundleOpts};
//! use provenance_dag::{Node, SealedDag, TenantKey};
//!
//! let k = TenantKey::from_bytes([7u8; 32]);
//! let mut d = SealedDag::new();
//! d.append(&k, Node::AgentAct { node_id: "SUP-A::sanction".into(), agent_role: "sanction_check".into(),
//!     inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(), output: "PASS".into(),
//!     evidence_refs: vec![], timestamp: 1 }).unwrap();
//! let input = BundleInput {
//!     records_jsonl: d.to_jsonl(), manifest_root: d.manifest_root(),
//!     report: "# SUP-A\n".into(), sealed_scope: "full-tenant-dag".into(), report_cases: vec!["SUP-A".into()],
//! };
//! let out = std::env::temp_dir().join("efe-s4-doctest-bundle");
//! let _ = std::fs::remove_dir_all(&out);
//! let m = assemble(&input, &k, &BundleOpts::default(), &out).unwrap();
//! assert!(out.join("verify/provenance-verify/src/main.rs").exists()); // source vendorée, re-compilable
//! assert!(out.join("README.md").exists());                            // le guide du sceptique
//! assert_eq!(m.n_records, 1);
//! let _ = std::fs::remove_dir_all(&out);
//! ```

pub mod bundle;
pub mod manifest;
pub mod readme;
pub mod vendor;

pub use bundle::{assemble, BundleInput, BundleManifest, BundleOpts, ExportError};
pub use manifest::{list_files, sha256_hex, write_manifest};
pub use readme::ReadmeCtx;
pub use vendor::{vendored_line_count, write_verify_src};
