// SPDX-License-Identifier: UNLICENSED
//! # `efe-swarm-sdk` — Phase 5 : l'API unifiée `swarm.run(task, topology, budget)`.
//!
//! Emballe les Phases 1-4 (substrat · topologies · consensus · provenance) en **une
//! fonction** : un dev lance un essaim **vérifiable, plafonné en coût, re-prouvable** —
//! en keyless ou self-host, **sans clé**.
//!
//! ```
//! use efe_swarm_sdk::{Swarm, TopologySpec, Budget};
//! use serde_json::json;
//!
//! # let rt = tokio::runtime::Runtime::new().unwrap();
//! # rt.block_on(async {
//! let swarm = Swarm::keyless();                          // keyless, provenance activée
//! let run = swarm.run(
//!     json!({ "text": "hello", "expected": "hello" }),
//!     TopologySpec::consensus("echo_generator", &["consistency", "length_band", "j_non_empty"], 5),
//!     Budget::cents(100),                                // plafond 1,00 $ — JAMAIS dépassé
//! ).await.unwrap();
//!
//! assert!(run.provenance.verified);                      // le run est re-prouvé (verify_swarm)
//! let _root = run.provenance.root.unwrap();              // un hash que ton régulateur vérifie
//! assert!(run.budget.spent_cents <= run.budget.max_cost_cents);
//! # });
//! ```
//!
//! ## Les 4 piliers (Inc.1)
//! - **`TopologySpec`** : T1 Generate-Verify · T2 Pipeline · T3 Hiérarchique · T-Consensus · **Auto**
//!   — agents résolus **par nom** depuis la flotte (Mode A, keyless, déterministe).
//! - **`ClientMode`** : Keyless / Self-host / Network — pilote le **modèle de prix** du
//!   budget et le mode de déploiement (**aucune clé** côté SDK).
//! - **`Budget`** + **3 politiques** (`Error`/`Stop`/`Degrade`) — plafond **jamais dépassé**
//!   (invariant I13 de `BudgetTracker`).
//! - **Provenance auto-câblée** : chaque run scelle une chaîne et renvoie un
//!   `provenance_root` re-vérifiable par un tiers (`verify_swarm`).
//!
//! > Inc.1 exécute la flotte **Mode A** (keyless, déterministe) — les 4 topologies
//! > tournent vraiment. Les agents **Mode C** (LLM via `ClientMode::SelfHost`/`Network`)
//! > sont déjà prouvés bout-en-bout dans
//! > `efe-swarm-provenance/examples/swarm_all_phases_real.rs` (Ollama local) ; leur
//! > câblage dans le SDK est la tranche suivante.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

mod agents;
mod llm;
mod resolver;
pub mod budget;
pub mod client;
pub mod observer;
pub mod result;
pub mod sdk;
pub mod spec;

pub use budget::{Budget, OnExceed};
pub use client::ClientMode;
pub use budget::BudgetSnapshot;
pub use observer::{AgentCall, SwarmObserver};
pub use result::{Accounting, ProvenanceSummary, QualitySummary, RunOutcome, SwarmError, SwarmRun};
pub use sdk::{Swarm, SwarmBuilder};
pub use spec::TopologySpec;

// Réexports pratiques (la passerelle HTTP s'appuie dessus pour persister + re-prouver).
pub use efe_swarm_provenance::{
    verify_swarm, AgentCallRecord, AgentKind, AgentRole, HmacKeySource, ProvenanceMetrics,
    SealedProvenance, SwarmProvenanceReport,
};
pub use efe_blackboard::{BlackboardWrite, EventSink, SubstrateMetrics, SwarmEvent, WriteSink};
pub use efe_swarm_topology::Pattern;

/// Version du crate (semver).
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");
