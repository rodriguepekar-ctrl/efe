// SPDX-License-Identifier: UNLICENSED
//! # `efe-multi-llm` — Phase 3 — Coordination multi-LLM
//!
//! Cette crate livre la coordination MULTI-LLM RÉELLE par-dessus le
//! Coordinator Phase 2 : 3 LLMs (Claude / GPT / Gemini) qui collaborent
//! coordonnés par les nano-robots Phase 1 + 2 nouveaux (Decomposer +
//! Aggregator).
//!
//! ## Les 5 missions
//!
//! | Mission | Fonction | Cas d'usage |
//! |---|---|---|
//! | [`workflows::run_vote_3llm`] | Fan-out 3 LLMs + reformulation pré-vote + Borda/Condorcet | Question factuelle où la diversité réduit l'hallucination |
//! | [`workflows::run_relay`] | Claude planifie → GPT code → Gemini synthétise (séquentiel typé) | Tâche complexe nécessitant plusieurs expertises |
//! | [`workflows::run_debate`] | 2 LLMs débattent (rounds + rebut) + 1 LLM jury | Décisions à fort enjeu, design ou éthique |
//! | [`workflows::run_specialist`] | Decomposer split → routing par sub-task → Aggregator merge | Requête complexe avec sub-tasks de natures différentes |
//! | [`workflows::run_cascade`] | Drafter local (free) → GPT verifier → Claude validator (rare) | Optimisation latence : -41% vs Claude solo |
//!
//! ## Composants
//!
//! | Module | Responsabilité |
//! |---|---|
//! | [`reformulator_ext`] | Convertit dialecte LLM (XML/JSON/MD) → canonical schemas v1.0.0 |
//! | [`vote_ext`] | Algos Borda + Condorcet (extension du Plurality Phase 2) |
//! | [`budget`] | `BudgetTracker` atomic avec try_charge + revert (invariant I13) |
//! | [`tenant`] | `TenantPheromoneBlackboard` isolation physique per-tenant (I12) |
//! | [`cancellation`] | `CancellationToken` pour early termination (I9) |
//! | [`decompose`] | `Decomposer` — découpe un prompt complexe en sub-tasks |
//! | [`aggregate`] | `Aggregator` — combine N canonical outputs en synthesis |
//! | [`canonical`] | Les 5 schemas frozen : plan, code, claim, argument, synthesis |
//!
//! ## Invariants Phase 3 (ajoutés au socle I1–I8 du Phase 2)
//!
//! - **I9** : early termination — si vote décidé avec n-1 LLMs, le n-ième
//!   est cancelé (économie ~33%).
//! - **I10** : audit chain complète — ≥ 1 entry par LLM + ≥ 1 par nano-robot.
//! - **I11** : anti-monoculture — aucun LLM ne représente > 60% des
//!   sub-tasks d'une mission Specialist.
//! - **I12** : isolation tenant — phéromones tenant A invisibles à tenant B.
//! - **I13** : budget hard-cap — try_charge atomic + revert si dépassement.
//!
//! ## Stabilité
//!
//! | Composant | Statut |
//! |---|---|
//! | 5 canonical schemas | **Frozen v1.0.0** |
//! | 5 fonctions `run_*` | **Stable** Phase 3 |
//! | `BudgetTracker` API | Stable |
//! | `vote_ext` Borda/Condorcet | Stable |
//! | `TenantPheromoneBlackboard` | Stable |
//! | `Decomposer` / `Aggregator` (heuristic mode) | Stable ; mode LLM-backed = Beta |

#![warn(
    missing_docs,
    rust_2018_idioms,
    unreachable_pub,
    elided_lifetimes_in_paths
)]
#![allow(clippy::module_name_repetitions)]

pub mod aggregate;
pub mod budget;
pub mod canonical;
pub mod cancellation;
pub mod decompose;
pub mod error;
pub mod events_ext;
pub mod reformulator_ext;
pub mod request;
pub mod tenant;
pub mod vote_ext;
pub mod workflows;

// Re-exports plats.
pub use aggregate::{Aggregator, AggregatorMode};
pub use budget::{BudgetSnapshot, BudgetTracker};
pub use canonical::{
    CanonicalArgument, CanonicalClaim, CanonicalCode, CanonicalPlan, CanonicalSchemaId,
    CanonicalSynthesis, CANONICAL_SCHEMA_VERSION,
};
pub use cancellation::{CancellationToken, CancellationReason};
pub use decompose::{Decomposer, DecomposerMode, SubTask};
pub use error::{MultiLlmError, MultiLlmResult};
pub use events_ext::Phase3EventExt;
pub use reformulator_ext::{
    reformulator_for, CanonicalReformulator, Reformulated, ReformulationHop, Reformulator,
    ReformulatorEngine, SlmHopAudit, SlmReformulator, SLM_REFORMULATOR_CIRCUIT_KEY,
};
pub use request::{
    CascadeRequest, CascadeResponse, DebateRequest, DebateResponse, RelayRequest,
    RelayResponse, SpecialistRequest, SpecialistResponse, VoteMode, Vote3LlmRequest,
    Vote3LlmResponse,
};
pub use tenant::TenantPheromoneBlackboard;
pub use vote_ext::{borda_tally, condorcet_tally, CondorcetOutcome, BordaOutcome};
pub use workflows::{
    run_cascade, run_debate, run_relay, run_specialist, run_vote_3llm,
};

/// Borne dure : n=3 est le sweet-spot Phase 3 (Symphony HF 2508.20019).
pub const DEFAULT_N_LLMS: usize = 3;

/// Anti-monoculture : un LLM ne peut représenter plus de X% des sub-tasks
/// dans une mission Specialist (invariant I11).
pub const ANTI_MONOCULTURE_MAX_RATIO: f32 = 0.60;

/// Round max d'un debate (au-delà, on tronque pour éviter le bavardage).
pub const DEBATE_MAX_ROUNDS: u8 = 5;

/// Sub-tasks max d'un specialist (au-delà, on rejette).
pub const SPECIALIST_MAX_SUB_TASKS: usize = 10;
