//! Fusion Kernels (M5) — swarm attention over the ELS.
//!
//! M5 is the computational core of each inference round. At round `r`,
//! it applies a fusion operator `F: ℝ^{N×D} → ℝ^{N×D}` over the latent
//! tensor of the swarm, driving the population toward consensus without
//! any text exchange.
//!
//! # Pipeline of one round
//!
//! 1. Validate config + pre-round stats.
//! 2. Snapshot latent into the replay buffer (M7 rollback).
//! 3. Acquire `ElsSplitMut` over the ELS (exclusive).
//! 4. Compute attention scores `s_ij = (h_i · h_j) / √D / T`.
//! 5. Row-wise softmax (max-shift stabilized).
//! 6. Weighted aggregation into a scratch buffer.
//! 7. Swap scratch ↔ ELS latent (pointer swap, zero copy).
//! 8. β-mix with the problem embedding.
//! 9. Project each row onto the alignment cone.
//! 10. Clip coordinates, detect NaN/Inf anomaly.
//! 11. Post-round stats + validation.
//!
//! # Public API
//!
//! - [`FusionStrategy`] — trait injected into the controller (M1).
//! - [`FusionConfig`] — campaign knobs.
//! - [`FusionRound`] — per-round outcome.
//! - [`FusionStats`] — scalar diagnostics consumed by M6 and M11.
//! - [`FusionWarning`] — non-fatal round issues.
//! - [`MvpCpuFusion`] — reference CPU implementation.
//! - [`ReplayBuffer`] — ring buffer of latent snapshots for M7.
//!
//! # Backward compatibility
//!
//! `FusionStats` keeps its legacy fields (`round`, `variance`, …) and
//! gains M5-specific attention-entropy fields. Synthetic constructors
//! default the new fields to zero so M3/M4 call sites keep compiling.

pub mod config;
/// **Sprint M1 extension D32 cross-cutting** — Hybrid CPU/GPU fusion
/// strategy for canary rollout. Cf.
/// `docs/EFE_CORE_EXTENSION_ARCHITECTURE.md` § 13.
pub mod hybrid;
pub(crate) mod kernels;
pub mod mvp_cpu;
pub mod replay;
pub mod round;
pub mod schedule;
pub mod stats;
pub mod strategy;
pub mod validator;

pub use config::FusionConfig;
pub use hybrid::{CanaryConfig, CanaryHybridFusion, HybridFusionReport};
pub use mvp_cpu::{MvpCpuFusion, MvpCpuFusionBuilder};
pub use replay::ReplayBuffer;
pub use round::{FusionRound, FusionWarning, RoundTiming};
pub use schedule::{alpha_for_round, beta_for_round};
pub use stats::FusionStats;
pub use strategy::{FusionStrategy, ReplayableFusion};
