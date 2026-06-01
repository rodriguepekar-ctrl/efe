//! Mode-3 multi-turn iterative reasoning (M22).
//!
//! Mode-3 = N appels Mode-2 chaînés avec convergence check après
//! chaque turn. Le pipeline LLMRouter (M21) est réutilisé sans
//! modification ; M22 ajoute uniquement :
//!   - [`ConceptStack`]      : trace de raisonnement turn × turn.
//!   - [`Mode3Engine`]       : orchestrateur multi-turn.
//!   - [`ReviewQueue`]       : Postgres-backed pending reviews.
//!   - [`Mode3Error`]        : typed errors mappable to gRPC codes.
//!
//! Garanties cardinales :
//!   - max_turns hard cap 5 (enforced à plusieurs niveaux).
//!   - concept_stack 100 % persisté pour GDPR Art. 22.
//!   - stakes_tier=high → human_review_required HARD GATE Postgres.
//!
//! Cf. `docs/EFE_M22_MODE3_MULTI_TURN_ARCHITECTURE.md`.

#![allow(dead_code)] // M22 Phase A scaffold ; full integration in Phase E
#![allow(missing_docs)] // M22 modules expose Pydantic-like data types
                        // ; field-level docs deferred to architecture
                        // doc EFE_M22_MODE3_MULTI_TURN_ARCHITECTURE.md

pub mod concept_stack;
pub mod engine;
pub mod error;
pub mod review_queue;
pub mod types;

pub use concept_stack::{ConceptStack, ConceptStackEntry};
pub use engine::{Mode3Engine, Mode3EngineConfig};
pub use error::Mode3Error;
pub use review_queue::{ReviewQueue, ReviewRecord, ReviewStatus};
pub use types::{
    ClassifyMode3Request, ClassifyMode3Response, ConvergenceResult, StakesTier, TenantMode3Config,
};

/// Hard cap on max_turns enforced at multiple levels :
/// - DB CHECK (tenant_feature_mode3.max_max_turns ≤ 5)
/// - Engine clamp (.min(MAX_TURNS_HARD_CAP))
/// - Request validation (max_turns ≤ 5)
pub const MAX_TURNS_HARD_CAP: u32 = 5;

/// Maximum length of `llm_reasoning` in concept_stack entries.
/// Truncation appended with "...[truncated]" for traceability.
pub const LLM_REASONING_MAX_CHARS: usize = 1024;

/// Prompt template id used for turn 0 (initial narrowing 10 → 5).
/// Reuses Mode-2 template (M21 carry-over).
pub const TEMPLATE_TURN_0: &str = "mode2_anchor_gen_v1";

/// Prompt template id used for turn 1+ (refinement N → 2).
/// New M22 template.
pub const TEMPLATE_REFINE: &str = "mode3_refine_v1";
