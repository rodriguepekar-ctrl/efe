//! **Sprint M1 extension D33** — Nano-agent infrastructure module.
//!
//! ## Mission
//!
//! Hosts the per-template behavioural descriptors required by the M1
//! GA contract (8 templates total : 3 MVP + 5 GA additions). Pre-D33
//! the only "template" surface was the `TemplateCode` enum in
//! [`crate::types`] ; this module adds :
//!
//!   1. The [`templates::NanoAgentTemplate`] trait — uniform contract
//!      for `default_share`, `fusion_kernel`, `consensus_weight`,
//!      `centroid_init`.
//!   2. One module per template (`na_sem.rs`, `na_log.rs`, `na_crit.rs`
//!      for MVP ; `na_qual.rs`, `na_causal.rs`, `na_temp.rs`,
//!      `na_safety.rs`, `na_consist.rs` for GA additions).
//!   3. The [`templates::FusionKernelId`] enum that fusion strategies
//!      dispatch on.
//!
//! ## Why a separate `agents/` directory
//!
//! The M1 architecture (cf. `EFE_CORE_EXTENSION_ARCHITECTURE.md` § 9.2)
//! mandates that templates live under `crates/efe-core/src/agents/`
//! rather than under `genesis/` because :
//!
//!   * `genesis/` is about *spawning* — the mechanics of how agents
//!     come into existence (DAG, cardinality, registry).
//!   * `agents/` is about *what an agent IS* — the behavioural
//!     descriptor that fusion + consensus consume.
//!
//! Separating them keeps the cognitive load on `genesis/` low and
//! gives templates first-class status in the public API surface.

pub mod templates;
