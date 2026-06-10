// SPDX-License-Identifier: UNLICENSED
//! # `efe-swarm-provenance` — Phase 4 : provenance **par-agent** (le *moat*).
//!
//! **Inc.1 — l'atome.** Ce crate rend traçable ce qui, aujourd'hui, est **muet** :
//! chaque appel d'un agent de l'essaim — y compris les agents **Mode A** (Rust pur,
//! le gros de la flotte) qui n'émettent aucun enregistrement d'appel-LLM.
//!
//! L'atome est [`AgentCallRecord`] — « **quel agent a dit quoi, quand, à partir de
//! quoi** » — hashé canoniquement et **vérifiable seul** :
//!
//! ```
//! use efe_swarm_provenance::{AgentCallDraft, AgentRole, AgentKind};
//! use efe_nano_sdk::{AgentId, SwarmId, TenantId};
//! use serde_json::json;
//!
//! // Un générateur Mode A produit le candidat 3, qu'il écrit en /cand/3@3.
//! let rec = AgentCallDraft::from_io(
//!     SwarmId::new(), TenantId::system(),
//!     AgentId::new("generator").unwrap(),
//!     AgentRole::Generator, AgentKind::ModeA,
//!     &json!({ "task": "classify", "difficulty": 0.7 }),  // À PARTIR DE QUOI
//!     &json!({ "value": "tech" }),                          // QUOI
//! )
//! .with_variant(3)
//! .wrote("/cand/3", 3)
//! .seal(0, None, [0u8; 32]); // position de chaîne (hmac posé par SwarmChain, Inc.2)
//!
//! assert_eq!(rec.agent_role, AgentRole::Generator);
//! assert!(rec.verify_payload_hash());            // l'atome est re-prouvable
//! ```
//!
//! ## Découpage du crate (Phase 4)
//! - **Inc.1** : [`record`] (l'atome), [`canon`] (JSON canonique cross-vérifiable),
//!   [`hash`] (BLAKE3 contenu + SHA-256 `payload_hash`).
//! - **Inc.2 (ici)** : [`chain`] — [`SwarmChain`] (HMAC par-swarm, genèse par
//!   `swarm_id`, tip = `provenance_root`) + [`verify_chain`] (vérificateur
//!   indépendant d'intégrité de chaîne). Crypto dans `efe-audit-core` (HMAC + HKDF
//!   par tenant, byte-identiques à `efe-nano`).
//! - **Inc.3** : capture par **2 seams sans réécrire les agents** —
//!   [`decorator`] ([`ProvenancedAgent`], seam B : lien entrée→sortie) +
//!   [`sink`] ([`ProvenanceSink`], seam A : ancrage 100 % des écritures) +
//!   [`recorder`] ([`ProvenanceRecorder`]) qui fusionne les deux en une [`SwarmChain`].
//! - **Inc.4 (ici, partiel)** : [`replay`] — rejeu **déterministe** clé sur la *version*
//!   du Blackboard, **pas** `SystemTime::now()` (ADR-SWARM-007). Même ensemble
//!   d'enregistrements → même `provenance_root`, quel que soit l'ordre d'exécution.
//! - **Pont N17** : [`bridge`] — relie un record d'essaim **Mode C** à son
//!   `LlmProvenanceRecord` (N17) via `llm_provenance_id` ([`LlmResolver`],
//!   [`verify_bridge`]). Une seule lignée, deux granularités.
//! - **Vérificateur indépendant** : [`verify`] — [`verify_swarm`] : un tiers, avec
//!   seulement les records + la clé tenant, re-prouve chaîne + contenu + DAG + couverture
//!   et reconstruit le `provenance_root`, **sans coopération du provider**. **Le moat.**
//! - **Observabilité (ici)** : [`metrics`] — [`ProvenanceMetrics`] : surfaces Prometheus
//!   sans dépendance (tracé, overhead, octets, ruptures). **Gate Lot P4** :
//!   `examples/lot_p4.rs` (overhead avec/sans provenance ±σ) → ADR-SWARM-006.
//!
//! ## Invariants gelés en Inc.1 (le contrat que la chaîne réutilisera)
//! - **Canonicalisation byte-identique** à `efe_nano::idempotency::canonicalize_json`
//!   (clés triées récursivement, aucun espace) ⇒ chaînes d'essaim ↔ chaînes de trace
//!   mutuellement vérifiables (§3.2 de l'architecture).
//! - **`payload_hash`** = SHA-256 du corps canonique (aligné `HMAC-SHA256` d'audit).
//! - **`input_hash`/`output_hash`** = BLAKE3-256 (aligné `efe-llm-provenance` N17).
//! - **`wall_at` advisory** : exclu du `payload_hash` (rejeu déterministe sur la
//!   *version*, pas l'horloge — ADR-SWARM-007).

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod bridge;
pub mod canon;
pub mod chain;
pub mod decorator;
pub mod hash;
pub mod metrics;
pub mod record;
pub mod recorder;
pub mod replay;
pub mod sink;
pub mod verify;

pub use bridge::{
    classify_link, resolve_llm, verify_bridge, BridgeLink, BridgeReport, LinkStatus, LlmResolver,
    LlmProvenanceView, MapResolver,
};
pub use canon::{canonical_bytes, canonical_json};
pub use chain::{verify_chain, BreakReason, ChainBreak, ChainVerdict, SwarmChain};
pub use decorator::ProvenancedAgent;
pub use hash::{blake3_bytes, blake3_value, payload_hash_of, sha256};
pub use metrics::{ProvenanceMetrics, ProvenanceMetricsSnapshot};
pub use record::{
    hex32, AgentCallDraft, AgentCallRecord, AgentKind, AgentRole, CellRef,
};
pub use recorder::{CallCapture, ProvenanceRecorder, SealedProvenance, WriteAnchor};
pub use replay::{
    deterministic_order, replay, replay_outputs, replay_root, OutputReplayReport, ReplayReport,
};
pub use sink::ProvenanceSink;
pub use verify::{verify_swarm, SwarmProvenanceReport};

// Réexport du socle crypto partagé (pratique pour l'appelant). `MASTER_ENV` = nom de la
// variable d'env du master HKDF (server-side) — utilisé par le binaire prod pour choisir
// `EnvHkdf` vs la clé de test, sans jamais lire la VALEUR du master.
pub use efe_audit_core::{chain_hmac, AuditKeyError, HmacKeySource, MASTER_ENV};

/// Version du crate (semver).
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");
