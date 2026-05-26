// SPDX-License-Identifier: UNLICENSED
//! # `efe-coordinator` — Phase 2 — DAG orchestrator du swarm nano-robots
//!
//! Cette crate est le **cerveau** de la suite nano-robots.  La Phase 1
//! ([`efe_nano_app_coordinator`]) a livré 6 clients Rust isolés qui parlent
//! à `llama-server` avec grammar enforcement (analyzer, reformulator,
//! compressor, verifier, filter, router).  La Phase 2 livre le
//! **Coordinator** — la pièce centrale qui les orchestre en **DAG parallèle**,
//! coordonne plusieurs **agents LLM gros en parallèle** (Claude/GPT/Gemini),
//! et expose tout cela en **streaming SSE** à un utilisateur final.
//!
//! ## Les deux missions cibles
//!
//! 1. **Mission A — pré-mâcher pour 1 gros LLM** ([`workflows::run_mission_a`])
//!    Analyzer → Filter ∥ Router ∥ Reformulator → Compressor → 1 LLM choisi
//!    → Verifier streaming.  Économise tokens et améliore qualité.
//!
//! 2. **Mission B — substrat info entre N gros LLMs** ([`workflows::run_mission_b`])
//!    Pré-mâchage unique → fan-out vers N LLMs en parallèle → vote pondéré
//!    Symphony → verifier global.  Donne une réponse consensus.
//!
//! 3. **Mission speculative** ([`workflows::run_speculative`])
//!    Speculative RAG : un SLM local drafte pendant qu'on attend le gros
//!    LLM ; le gros LLM agit comme *verifier* à la fin.  Réduit la latence
//!    perçue de ~40 %.
//!
//! ## Composants principaux
//!
//! | Module | Responsabilité |
//! |---|---|
//! | [`coordinator::Coordinator`] | Surface publique : `execute_mission_a/b/speculative` |
//! | [`dag::DagExecutor`] | Orchestrateur DAG (fan-out / fan-in via `tokio::JoinSet`) |
//! | [`blackboard::PheromoneBlackboard`] | Stigmergie bornée 256 Ko (SwarmSys-inspired) |
//! | [`events::EventBus`] | Pub/sub abstraction : `InMemoryBus` + `NatsBus` (feature) |
//! | [`events::StreamEvent`] | Enum typé des événements SSE exposés à l'UI |
//! | [`llm::MultiLlmBridge`] | Fan-out unifié Claude/GPT-4/Gemini/local |
//! | [`vote::VoteEngine`] | Vote pondéré Symphony (HF 2508.20019) |
//! | [`speculate::Speculator`] | Pattern drafter/verifier (HF 2407.08223) |
//!
//! ## Architecture en couches
//!
//! ```text
//!     ┌──────────────────────────────────────────────────────────────┐
//!     │  USAGE                                                        │
//!     │  let coord = Coordinator::builder().build()?;                 │
//!     │  let mut stream = coord.execute_mission_a(req, &ctx).await?;  │
//!     │  while let Some(ev) = stream.next().await { ui.push(ev) }     │
//!     └──────────────────────────┬───────────────────────────────────┘
//!                                ▼
//!     ┌──────────────────────────────────────────────────────────────┐
//!     │  Coordinator + workflows::*                                   │
//!     │  Mission A: analyzer → filter∥router∥reformulator → compr…   │
//!     │  Mission B: pre-chew → fan-out N LLMs → vote → verifier      │
//!     │  Speculative: drafter pendant la frappe → verifier           │
//!     └──────────────────────────┬───────────────────────────────────┘
//!                                ▼
//!     ┌──────────────────────────────────────────────────────────────┐
//!     │  DagExecutor + PheromoneBlackboard + EventBus                 │
//!     │  parallel tokio::JoinSet • bounded 256 KB • SSE pub/sub      │
//!     └──────────────────────────┬───────────────────────────────────┘
//!                                ▼
//!     ┌──────────────────────────────────────────────────────────────┐
//!     │  Phase 1: NanoRobotClient (analyzer/filter/router/…)         │
//!     │  Phase 2: MultiLlmBridge (claude/gpt4/gemini/…)              │
//!     └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Invariants
//!
//! - **I1** : chaque invocation de robot produit exactement 1 audit entry
//!   (succès, échec, timeout — toujours 1, jamais 0 ou 2).
//! - **I2** : la blackboard ne dépasse jamais 256 Ko ([`blackboard::MAX_BLACKBOARD_BYTES`]).
//! - **I3** : tout `StreamEvent` est sérialisable JSON et roundtrip-stable.
//! - **I4** : les workflows sont idempotents pour un `swarm_id` donné.
//! - **I5** : un vote sans majorité retombe sur le LLM ayant le plus haut
//!   `success_rate` historique (pas d'aléa).
//! - **I6** : `Coordinator::execute_mission_b` exige ≥ 2 LLMs ; sinon
//!   bascule automatiquement vers Mission A (`MissionFallback::ToMissionA`).
//! - **I7** : les phéromones décroissent monotone (jamais ré-augmentent
//!   sans nouveau dépôt).
//! - **I8** : le `DagExecutor` borne le parallélisme à `max_concurrency`
//!   (configurable, défaut 8) — pas de meltdown sur swarm large.
//!
//! ## Stabilité
//!
//! | Composant | Statut |
//! |---|---|
//! | [`coordinator::Coordinator`] (surface publique) | **Stable** Phase 2 |
//! | [`events::StreamEvent`] (protocole SSE) | **Stable v1.0.0** |
//! | [`blackboard::PheromoneBlackboard`] | Stable |
//! | [`vote::VoteEngine`] | Stable |
//! | [`speculate::Speculator`] | Stable |
//! | [`llm::MultiLlmBridge`] (trait) | Stable |
//! | [`events::NatsBus`] (feature `nats-bus`) | Beta — branchement Phase 7 |

#![warn(
    missing_docs,
    rust_2018_idioms,
    unreachable_pub,
    elided_lifetimes_in_paths
)]
#![allow(clippy::module_name_repetitions, clippy::needless_pass_by_value)]

pub mod audit;
pub mod blackboard;
pub mod coordinator;
pub mod dag;
pub mod error;
pub mod events;
pub mod heartbeat;
pub mod llm;
pub mod request;
pub mod speculate;
pub mod sse;
pub mod vote;
pub mod workflows;

// Re-exports plats — surface publique stable.
pub use audit::{ChainedEntry, HmacAuditChain, DEFAULT_AUDIT_KEY, GENESIS_HASH};
pub use blackboard::{
    BlackboardEntry, EntryKind, PheromoneBlackboard, MAX_BLACKBOARD_BYTES, MAX_ENTRIES,
};
pub use heartbeat::{HeartbeatHandle, DEFAULT_HEARTBEAT_PERIOD};
pub use sse::{parse_sse_blob, to_sse_blob, SseByteStream, CONTENT_TYPE_SSE, SSE_PREFACE};
pub use coordinator::{Coordinator, CoordinatorBuilder, CoordinatorClients, CoordinatorConfig};
pub use dag::{DagExecutor, DagNode, NodeId, NodeStatus, ParentOutputs};
pub use error::{CoordError, CoordResult};
pub use events::{
    filter_and, filter_by_swarm, filter_by_tag, filter_or, EventBus, EventFilter,
    InMemoryBus, MissionTag, StreamEvent, SubscriberToken, EVENT_SCHEMA_VERSION,
};
pub use llm::{
    LlmCall, LlmProvider, LlmResponse, MockLlmProvider, MultiLlmBridge, ProviderHandle,
    ProviderRegistry,
};

#[cfg(feature = "anthropic")]
pub use llm::ClaudeProvider;
#[cfg(feature = "gemini")]
pub use llm::GeminiProvider;
#[cfg(any(feature = "anthropic", feature = "gemini"))]
pub use llm::providers::{strip_thinking_tags, PiiRedactor};
#[cfg(feature = "anthropic")]
pub use llm::providers::ClaudeRateLimits;
#[cfg(feature = "gemini")]
pub use llm::providers::GeminiObservability;
pub use request::{
    CandidateSourceRef, FactCheck, MissionAResponse, MissionARequest, MissionBRequest,
    MissionBResponse, SpeculativeRequest, SpeculativeResponse,
};
pub use speculate::{DraftToken, SpeculationOutcome, Speculator};
pub use vote::{Ballot, VoteEngine, VoteOutcome};
pub use workflows::{run_mission_a, run_mission_b, run_speculative};

/// Version du protocole SSE (`StreamEvent`).  À aligner avec le client UI.
pub const SSE_PROTOCOL_VERSION: &str = "1.0.0";

/// Borne dure du nombre de LLMs gros qu'un Coordinator peut adresser
/// en parallèle (mission B + speculative confondus).
pub const MAX_PARALLEL_LLMS: usize = 8;

/// Borne dure du parallélisme du DAG executor par défaut.
pub const DEFAULT_MAX_CONCURRENCY: usize = 8;
