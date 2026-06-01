//! **Sprint M1 extension D33** — Nano-agent template trait + 8 impls.
//!
//! ## Architecture
//!
//! Cf. `docs/EFE_CORE_EXTENSION_ARCHITECTURE.md` § 9.
//!
//! Each template is a unit struct that implements [`NanoAgentTemplate`].
//!
//! ## Wiring status (post-M1 audit)
//!
//! **Audit fix C-A5** — the trait surface is fully populated but
//! only one method (`default_share()`) is currently consumed by the
//! existing pipeline (via [`crate::genesis::cardinality::balanced_ga`]).
//! The remaining methods are **declared but not yet wired** into the
//! algorithmic engines :
//!
//! | Method | Consumer (target) | Status |
//! |--------|-------------------|--------|
//! | `default_share()` | `genesis/cardinality::balanced_ga` | ✅ **WIRED** (D33) |
//! | `fusion_kernel()` | `fusion::mvp_cpu::MvpCpuFusion` per-template kernel dispatch | 🔧 `DT-C-D33-FUSION-KERNEL-1` |
//! | `consensus_weight()` | `consensus::detector::MultiCriteriaDetector` weighted voting | 🔧 `DT-C-D33-CONSENSUS-WEIGHT-1` |
//! | `centroid_init()` | `genesis::spawner` per-agent init rule | 🔧 `DT-C-D33-CENTROID-INIT-1` |
//!
//! The "declared but not wired" surface is intentional — wiring each
//! consumer requires careful refactoring of the existing CPU MVP
//! pipeline (≥ 1 021 unit tests to not break) and is out of scope
//! for the M1 extension D33 ship. Each consumer's wiring is tracked
//! by its own DT identifier and will land in a follow-up audit pass.
//!
//! Pre-D33 callers that read these methods do so **at their own
//! risk** — the values are honoured by the trait but not propagated
//! into the swarm execution pipeline. For example, `NaSafety`'s
//! `consensus_weight() = 2.0` is currently NOT applied by the
//! consensus detector (the detector treats every template as
//! weight 1.0 internally). Operators that rely on the safety boost
//! must wait for `DT-C-D33-CONSENSUS-WEIGHT-1`.
//!
//! ## Backward compatibility
//!
//! The 3 MVP templates (`NaSem`, `NaLog`, `NaCrit`) are extracted
//! from `genesis/registry.rs` (where their behavior was hardcoded
//! pre-D33) into dedicated module files. The behavior is preserved
//! byte-for-byte ; the refactor is purely organisational. Pre-D33
//! pipeline behaviour with the 3 MVP templates is unchanged.

use crate::types::TemplateCode;

pub mod na_causal;
pub mod na_consist;
pub mod na_crit;
pub mod na_log;
pub mod na_qual;
pub mod na_safety;
pub mod na_sem;
pub mod na_temp;

pub use na_causal::NaCausal;
pub use na_consist::NaConsist;
pub use na_crit::NaCrit;
pub use na_log::NaLog;
pub use na_qual::NaQual;
pub use na_safety::NaSafety;
pub use na_sem::NaSem;
pub use na_temp::NaTemp;

// =====================================================================
// Trait
// =====================================================================

/// **Sprint D33** — Per-template behavioural descriptor.
///
/// Every template (MVP + GA additions) implements this trait. Cf.
/// the **Wiring status** table at the top of this module for the
/// current consumption status of each method.
///
///   * `default_share()` — **currently wired** in
///     [`crate::genesis::cardinality::balanced_ga`].
///   * `fusion_kernel()` — declared, **not yet wired** into the
///     fusion engine. Tracked by `DT-C-D33-FUSION-KERNEL-1`.
///   * `consensus_weight()` — declared, **not yet wired** into the
///     consensus detector. Tracked by `DT-C-D33-CONSENSUS-WEIGHT-1`.
///   * `centroid_init()` — declared, **not yet wired** into the
///     genesis spawner. Tracked by `DT-C-D33-CENTROID-INIT-1`.
///
/// Trait bounds : `Send + Sync + Debug + 'static` so trait objects
/// can live in the global template registry behind `Arc<dyn NanoAgentTemplate>`.
pub trait NanoAgentTemplate: Send + Sync + std::fmt::Debug + 'static {
    /// Stable enum variant — the unique identity of this template.
    fn code(&self) -> TemplateCode;

    /// Default share in a balanced GA population mix.
    ///
    /// The sum across all GA templates should equal `1.0`. The
    /// invariant is checked at compile time by
    /// [`crate::genesis::cardinality::PopulationMix::balanced_ga`].
    /// Individual values are tuned by the data team based on
    /// real-world workload measurements (D33 baseline).
    fn default_share(&self) -> f32;

    /// Fusion kernel routing key. Returns the kernel id that
    /// `MvpCpuFusion` (and friends) must dispatch to for nano-agents
    /// of this template.
    fn fusion_kernel(&self) -> FusionKernelId;

    /// Weight in the multi-criteria consensus detector.
    ///
    /// `1.0` = baseline (template's vote counts as 1). Higher = the
    /// template's votes are more influential ; lower = less. Defaults
    /// to `1.0` to preserve backward compat for any future template
    /// that doesn't override.
    ///
    /// **Production-tuned values** (D33 baseline) :
    ///   * `NaSafety` = 2.0 (refusal trumps positive votes)
    ///   * `NaConsist` = 1.5 (consistency boosts confidence)
    ///   * `NaQual` = 1.2 (quality matters but doesn't dominate)
    ///   * Everything else = 1.0
    fn consensus_weight(&self) -> f32 {
        1.0
    }

    /// Optional override of the centroid initialisation rule for
    /// nano-agents of this template. Returns `None` to use the default
    /// (perturb-problem-embedding with a Gaussian sigma). A few
    /// templates (e.g. `NaSafety`) want a fixed centroid in the
    /// "refusal" direction of the latent space.
    fn centroid_init(&self) -> Option<CentroidInit> {
        None
    }

    /// Human-readable description used by the `efe-onboard` CLI when
    /// it prints a catalog's template manifest. Defaults to the
    /// template code's `as_str()` value.
    fn description(&self) -> &'static str {
        self.code().as_str()
    }
}

// =====================================================================
// Fusion kernel routing
// =====================================================================

/// **Sprint D33** — Routing key consumed by fusion strategies to
/// dispatch the right kernel per template.
///
/// Each `TemplateCode` maps to one `FusionKernelId` via
/// [`NanoAgentTemplate::fusion_kernel`]. The fusion engine indexes its
/// per-kernel weight tensors / activation functions by this enum.
///
/// **Stability** : `#[non_exhaustive]` so future templates (post-D33)
/// can add kernels without breaking pattern-matching consumers ;
/// existing consumers must always have a `_ => default` arm.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FusionKernelId {
    /// Standard semantic kernel (MVP, NaSem default).
    Semantic,
    /// Logical-inference kernel (MVP, NaLog default).
    Logical,
    /// Diversification kernel (MVP, NaCrit default).
    Diversifying,
    // ────────────────────────────────────────────────────────────
    // Sprint D33 — GA kernels
    // ────────────────────────────────────────────────────────────
    /// Quality-scoring kernel (D33, NaQual).
    Quality,
    /// Causal-graph kernel (D33, NaCausal).
    Causal,
    /// Temporal-attention kernel (D33, NaTemp).
    Temporal,
    /// Safety-classification kernel (D33, NaSafety).
    Safety,
    /// Consistency-check kernel (D33, NaConsist).
    Consistency,
}

impl FusionKernelId {
    /// Stable string id for telemetry labels and dashboards.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Logical => "logical",
            Self::Diversifying => "diversifying",
            Self::Quality => "quality",
            Self::Causal => "causal",
            Self::Temporal => "temporal",
            Self::Safety => "safety",
            Self::Consistency => "consistency",
        }
    }
}

// =====================================================================
// Centroid initialisation
// =====================================================================

/// **Sprint D33** — Optional centroid init rule per template.
///
/// Returned by [`NanoAgentTemplate::centroid_init`]. Genesis consumes
/// this to spawn each nano-agent at the right starting position in
/// latent space.
#[derive(Clone, Debug)]
pub enum CentroidInit {
    /// Take `problem_embedding + Gaussian(0, sigma)`. The default
    /// behaviour for templates that don't override.
    PerturbProblem {
        /// Standard deviation of the Gaussian perturbation. Typical
        /// values : 0.05 – 0.15.
        sigma: f32,
    },
    /// Take a fixed point in latent space. Length MUST match
    /// `dim_latent` of the active `SwarmSpec`. Used by `NaSafety`
    /// to anchor refusal-direction agents.
    Fixed(Vec<f32>),
    /// Sample uniformly from `[-1, 1]^dim`. Cheap, decorrelating ;
    /// used by `NaCrit` (the diversification template) so its agents
    /// explore the whole space.
    Uniform,
}

// =====================================================================
// Convenience helpers
// =====================================================================

/// Return the `Arc<dyn NanoAgentTemplate>` for every GA template, in
/// the canonical order defined by [`TemplateCode::GA_SET`].
///
/// **Sprint D33** — utility used by the genesis engine to register
/// every template at boot. Consumers that only need the MVP set can
/// filter via `code().is_mvp()`.
#[must_use]
pub fn ga_templates() -> Vec<std::sync::Arc<dyn NanoAgentTemplate>> {
    use std::sync::Arc;
    vec![
        Arc::new(NaSem) as Arc<dyn NanoAgentTemplate>,
        Arc::new(NaLog) as Arc<dyn NanoAgentTemplate>,
        Arc::new(NaCrit) as Arc<dyn NanoAgentTemplate>,
        Arc::new(NaQual) as Arc<dyn NanoAgentTemplate>,
        Arc::new(NaCausal) as Arc<dyn NanoAgentTemplate>,
        Arc::new(NaTemp) as Arc<dyn NanoAgentTemplate>,
        Arc::new(NaSafety) as Arc<dyn NanoAgentTemplate>,
        Arc::new(NaConsist) as Arc<dyn NanoAgentTemplate>,
    ]
}

/// Return the MVP-only template set for backward compat.
#[must_use]
pub fn mvp_templates() -> Vec<std::sync::Arc<dyn NanoAgentTemplate>> {
    use std::sync::Arc;
    vec![
        Arc::new(NaSem) as Arc<dyn NanoAgentTemplate>,
        Arc::new(NaLog) as Arc<dyn NanoAgentTemplate>,
        Arc::new(NaCrit) as Arc<dyn NanoAgentTemplate>,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin test : sum of GA default_share is approximately 1.0.
    /// Slight tolerance for f32 round-off.
    #[test]
    fn ga_default_shares_sum_to_unity() {
        let sum: f32 = ga_templates().iter().map(|t| t.default_share()).sum();
        assert!(
            (sum - 1.0).abs() < 0.005,
            "GA default_share sum = {sum} ; expected ≈ 1.0",
        );
    }

    /// Pin test : every TemplateCode variant has exactly one
    /// template impl in `ga_templates()`.
    #[test]
    fn ga_templates_cover_every_template_code() {
        let codes: std::collections::HashSet<TemplateCode> =
            ga_templates().iter().map(|t| t.code()).collect();
        for &code in TemplateCode::GA_SET {
            assert!(codes.contains(&code), "missing template impl for {code:?}");
        }
        assert_eq!(codes.len(), TemplateCode::GA_SET.len());
    }

    /// Pin test : MVP templates all return `is_mvp() == true`.
    #[test]
    fn mvp_templates_are_mvp() {
        for t in mvp_templates() {
            assert!(t.code().is_mvp(), "{:?} is in MVP set but is_mvp() = false", t);
        }
    }

    /// Pin test : GA-only templates return `is_mvp() == false`.
    #[test]
    fn ga_only_templates_are_not_mvp() {
        let mvp_codes: std::collections::HashSet<TemplateCode> =
            mvp_templates().iter().map(|t| t.code()).collect();
        for t in ga_templates() {
            if !mvp_codes.contains(&t.code()) {
                assert!(
                    !t.code().is_mvp(),
                    "{:?} is in GA-only but is_mvp() = true",
                    t
                );
            }
        }
    }

    /// Pin test : each template has a distinct fusion_kernel id.
    /// **Audit fix C-A1** — the MVP and GA kernels must not collide ;
    /// otherwise the fusion engine would silently dispatch the same
    /// kernel for two different templates.
    #[test]
    fn fusion_kernels_are_distinct_per_template() {
        let kernels: std::collections::HashSet<FusionKernelId> =
            ga_templates().iter().map(|t| t.fusion_kernel()).collect();
        assert_eq!(
            kernels.len(),
            TemplateCode::GA_SET.len(),
            "fusion kernels must be 1-to-1 with templates"
        );
    }

    /// Pin test : NaSafety has the highest consensus weight (2.0).
    /// Institutional invariant : a refusal vote should be the
    /// strongest signal in the swarm.
    #[test]
    fn na_safety_has_highest_consensus_weight() {
        let safety = NaSafety;
        let safety_weight = safety.consensus_weight();
        for t in ga_templates() {
            if t.code() != TemplateCode::NaSafety {
                assert!(
                    t.consensus_weight() <= safety_weight,
                    "{:?} weight {} > NaSafety {}",
                    t.code(),
                    t.consensus_weight(),
                    safety_weight,
                );
            }
        }
        assert!((safety_weight - 2.0).abs() < 1e-6);
    }
}
