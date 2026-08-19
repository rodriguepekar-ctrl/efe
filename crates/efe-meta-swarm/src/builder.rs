//! Fluent builder for [`crate::MetaSwarm`].
//!
//! All validation (dimensions consistent, alphas + gamma < 1, lifts and
//! pulls matching meta-space, etc.) is performed in [`MetaSwarmBuilder::build`].
//!
//! ## Forward-compat
//!
//! The builder accepts an `Option<TenantId>` (Phase-3 binding) and an
//! optional [`crate::observability::MetaSwarmTracer`] (Phase-3
//! observability).  Both default to `None` and `NoopTracer`.

use std::collections::HashMap;

use crate::{MetaSwarmError, Result, MetaSwarm, TenantId};
use crate::iteration::ConvergenceCriterion;
use crate::operators::{Lift, Pull, MetaOperator, UniformMetaOp};
use crate::participants::SwarmParticipant;
use crate::safety::{SafetySet, BoxSafety};
use crate::observability::{MetaSwarmTracer, NoopTracer};

/// Fluent builder for a `MetaSwarm`.
pub struct MetaSwarmBuilder {
    id: Option<String>,
    meta_dim: Option<usize>,
    participants: Vec<Box<dyn SwarmParticipant>>,
    lifts_by_id: HashMap<String, Box<dyn Lift>>,
    pulls_by_id: HashMap<String, Box<dyn Pull>>,
    alphas_by_id: HashMap<String, f64>,
    default_alpha: f64,
    meta_op: Option<Box<dyn MetaOperator>>,
    meta_safety: Option<Box<dyn SafetySet>>,
    gamma: f64,
    criterion: ConvergenceCriterion,
    trace_enabled: bool,
    /// Couche 3 M1.c — fail-closed mode (default off for Phase-1 backward-compat).
    strict: bool,
    tenant_id: Option<TenantId>,
    tracer: Option<Box<dyn MetaSwarmTracer>>,
}

impl Default for MetaSwarmBuilder {
    fn default() -> Self {
        Self {
            id: None,
            meta_dim: None,
            participants: Vec::new(),
            lifts_by_id: HashMap::new(),
            pulls_by_id: HashMap::new(),
            alphas_by_id: HashMap::new(),
            default_alpha: 0.5,
            meta_op: None,
            meta_safety: None,
            gamma: 0.2,
            criterion: ConvergenceCriterion::default(),
            trace_enabled: false,
            strict: false,
            tenant_id: None,
            tracer: None,
        }
    }
}

impl MetaSwarmBuilder {
    /// Set the meta-swarm identifier.
    #[must_use]
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Set the meta-space dimension.
    #[must_use]
    pub fn meta_space_dim(mut self, dim: usize) -> Self {
        self.meta_dim = Some(dim);
        self
    }

    /// Add a participant.  Subsequent calls must reference its id when
    /// adding the matching lift/pull.
    #[must_use]
    pub fn add_participant(mut self, p: Box<dyn SwarmParticipant>) -> Self {
        self.participants.push(p);
        self
    }

    /// Set the lift for a previously-added participant.
    #[must_use]
    pub fn lift_for(mut self, participant_id: impl Into<String>, lift: Box<dyn Lift>) -> Self {
        self.lifts_by_id.insert(participant_id.into(), lift);
        self
    }

    /// Set the pull for a previously-added participant.
    #[must_use]
    pub fn pull_for(mut self, participant_id: impl Into<String>, pull: Box<dyn Pull>) -> Self {
        self.pulls_by_id.insert(participant_id.into(), pull);
        self
    }

    /// Override the alpha for a specific participant.
    #[must_use]
    pub fn alpha_for(mut self, participant_id: impl Into<String>, alpha: f64) -> Self {
        self.alphas_by_id.insert(participant_id.into(), alpha);
        self
    }

    /// Set the default alpha for participants that don't have an override.
    #[must_use]
    pub fn alpha_default(mut self, alpha: f64) -> Self {
        self.default_alpha = alpha;
        self
    }

    /// Set the meta-operator.
    #[must_use]
    pub fn meta_operator(mut self, op: Box<dyn MetaOperator>) -> Self {
        self.meta_op = Some(op);
        self
    }

    /// Set the meta-safety set.
    #[must_use]
    pub fn meta_safety(mut self, safety: Box<dyn SafetySet>) -> Self {
        self.meta_safety = Some(safety);
        self
    }

    /// Set the coupling coefficient $\gamma$.
    #[must_use]
    pub fn coupling_gamma(mut self, gamma: f64) -> Self {
        self.gamma = gamma;
        self
    }

    /// Set the convergence tolerance.
    #[must_use]
    pub fn tolerance(mut self, tol: f64) -> Self {
        self.criterion.tolerance = tol;
        self
    }

    /// Set the maximum number of iterations.
    #[must_use]
    pub fn max_iters(mut self, n: usize) -> Self {
        self.criterion.max_iters = n;
        self
    }

    /// **Couche 3 M1.c** — enable *strict* (fail-closed) mode. When on:
    ///   * the H1 guard **rejects** a participant that returns a
    ///     [`crate::participants::NonExpansiveCertificate::Stub`] (an unverified
    ///     participant — e.g. a `MockParticipant` — can no longer silently pass);
    ///   * a run that hits `max_iters` without converging returns
    ///     [`MetaSwarmError::ConvergenceFailed`] instead of a non-converged consensus
    ///     (never claim "consensus" when the iteration did not converge).
    ///
    /// Off by default so Phase-1 mock demos keep working. A `LocalParticipant` is
    /// checked against its real empirical canary in *both* modes.
    #[must_use]
    pub fn strict(mut self, on: bool) -> Self {
        self.strict = on;
        self
    }

    /// Enable iteration tracing (memory-intensive ; off by default).
    #[must_use]
    pub fn trace(mut self, on: bool) -> Self {
        self.trace_enabled = on;
        self
    }

    /// Forward-compat — bind to a tenant.  Phase 1 stores but ignores.
    #[must_use]
    pub fn tenant_id(mut self, t: TenantId) -> Self {
        self.tenant_id = Some(t);
        self
    }

    /// Forward-compat — install a tracer.  Phase 1 defaults to `NoopTracer`.
    #[must_use]
    pub fn tracer(mut self, t: Box<dyn MetaSwarmTracer>) -> Self {
        self.tracer = Some(t);
        self
    }

    /// Validate and assemble.
    pub fn build(self) -> Result<MetaSwarm> {
        let id = self.id.ok_or(MetaSwarmError::Internal("MetaSwarmBuilder: missing id".into()))?;
        let meta_dim = self.meta_dim.ok_or(MetaSwarmError::Internal(
            "MetaSwarmBuilder: missing meta_space_dim".into()))?;
        if self.participants.is_empty() {
            return Err(MetaSwarmError::EmptyMetaSwarm);
        }

        // Validate unique ids.
        let mut seen = std::collections::HashSet::new();
        for p in &self.participants {
            let pid = p.id().to_string();
            if !seen.insert(pid.clone()) {
                return Err(MetaSwarmError::DuplicateParticipantId { id: pid });
            }
        }

        // Each participant must have a lift and a pull.
        let mut lifts: Vec<Box<dyn Lift>> = Vec::with_capacity(self.participants.len());
        let mut pulls: Vec<Box<dyn Pull>> = Vec::with_capacity(self.participants.len());
        let mut alphas: Vec<f64> = Vec::with_capacity(self.participants.len());
        let mut lifts_remaining = self.lifts_by_id;
        let mut pulls_remaining = self.pulls_by_id;
        for p in &self.participants {
            let pid = p.id().to_string();
            let lift = lifts_remaining.remove(&pid)
                .ok_or(MetaSwarmError::MissingOperator { id: pid.clone(), missing: "lift" })?;
            if lift.from_dim() != p.dim() {
                return Err(MetaSwarmError::DimensionMismatch {
                    expected: p.dim(),
                    actual:   lift.from_dim(),
                });
            }
            if lift.to_dim() != meta_dim {
                return Err(MetaSwarmError::DimensionMismatch {
                    expected: meta_dim,
                    actual:   lift.to_dim(),
                });
            }
            let pull = pulls_remaining.remove(&pid)
                .ok_or(MetaSwarmError::MissingOperator { id: pid.clone(), missing: "pull" })?;
            if pull.from_dim() != meta_dim {
                return Err(MetaSwarmError::DimensionMismatch {
                    expected: meta_dim,
                    actual:   pull.from_dim(),
                });
            }
            if pull.to_dim() != p.dim() {
                return Err(MetaSwarmError::DimensionMismatch {
                    expected: p.dim(),
                    actual:   pull.to_dim(),
                });
            }
            lifts.push(lift);
            pulls.push(pull);
            let alpha = *self.alphas_by_id.get(&pid).unwrap_or(&self.default_alpha);
            if alpha + self.gamma >= 1.0 {
                return Err(MetaSwarmError::InvalidCoupling { alpha, gamma: self.gamma });
            }
            alphas.push(alpha);
        }

        // Sanity : extraneous lifts/pulls referencing unknown participants.
        if let Some(stray) = lifts_remaining.keys().next() {
            return Err(MetaSwarmError::ParticipantNotFound { id: stray.clone() });
        }
        if let Some(stray) = pulls_remaining.keys().next() {
            return Err(MetaSwarmError::ParticipantNotFound { id: stray.clone() });
        }

        let meta_op = self.meta_op.unwrap_or_else(|| Box::new(UniformMetaOp::new(meta_dim)));
        if meta_op.meta_dim() != meta_dim {
            return Err(MetaSwarmError::DimensionMismatch {
                expected: meta_dim,
                actual:   meta_op.meta_dim(),
            });
        }
        let meta_safety = self.meta_safety.unwrap_or_else(|| Box::new(BoxSafety::symmetric(meta_dim, 10.0)));
        if meta_safety.dim() != meta_dim {
            return Err(MetaSwarmError::DimensionMismatch {
                expected: meta_dim,
                actual:   meta_safety.dim(),
            });
        }

        let tracer = self.tracer.unwrap_or_else(|| Box::new(NoopTracer));

        // Note : `tenant_id` is captured but unused in Phase 1.  We
        // silence the warning explicitly.
        let _ = self.tenant_id;

        Ok(MetaSwarm {
            id,
            participants: self.participants,
            lifts,
            pulls,
            meta_op,
            meta_safety,
            alphas,
            gamma: self.gamma,
            criterion: self.criterion,
            trace_enabled: self.trace_enabled,
            strict: self.strict,
            tracer,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operators::{LinearLift, LinearPull, UniformMetaOp};
    use crate::participants::MockParticipant;

    fn small_meta_swarm() -> Result<MetaSwarm> {
        MetaSwarm::builder()
            .id("test")
            .meta_space_dim(4)
            .add_participant(MockParticipant::with_dim("a", 8).boxed())
            .add_participant(MockParticipant::with_dim("b", 8).boxed())
            .lift_for("a", LinearLift::identity(8, 4)?.boxed())
            .lift_for("b", LinearLift::identity(8, 4)?.boxed())
            .pull_for("a", LinearPull::identity(4, 8)?.boxed())
            .pull_for("b", LinearPull::identity(4, 8)?.boxed())
            .meta_operator(UniformMetaOp::new(4).boxed())
            .meta_safety(BoxSafety::symmetric(4, 5.0).boxed())
            .alpha_default(0.5)
            .coupling_gamma(0.2)
            .tolerance(1e-4)
            .max_iters(500)
            .build()
    }

    #[test]
    fn small_meta_swarm_builds() {
        let ms = small_meta_swarm().unwrap();
        assert_eq!(ms.id(), "test");
        assert_eq!(ms.m(), 2);
    }

    #[test]
    fn empty_meta_swarm_is_rejected() {
        let r = MetaSwarm::builder()
            .id("empty")
            .meta_space_dim(4)
            .meta_operator(UniformMetaOp::new(4).boxed())
            .meta_safety(BoxSafety::symmetric(4, 5.0).boxed())
            .build();
        match r {
            Err(MetaSwarmError::EmptyMetaSwarm) => (),
            other => panic!("expected EmptyMetaSwarm, got {other:?}"),
        }
    }

    #[test]
    fn missing_lift_is_reported() {
        let r = MetaSwarm::builder()
            .id("test")
            .meta_space_dim(4)
            .add_participant(MockParticipant::with_dim("a", 8).boxed())
            .pull_for("a", LinearPull::identity(4, 8).unwrap().boxed())
            .build();
        match r {
            Err(MetaSwarmError::MissingOperator { missing: "lift", .. }) => (),
            other => panic!("expected MissingOperator(lift), got {other:?}"),
        }
    }

    #[test]
    fn dim_mismatch_lift_is_reported() {
        let r = MetaSwarm::builder()
            .id("test")
            .meta_space_dim(4)
            .add_participant(MockParticipant::with_dim("a", 8).boxed())
            .lift_for("a", LinearLift::identity(8, 6).unwrap().boxed())  // wrong out dim
            .pull_for("a", LinearPull::identity(4, 8).unwrap().boxed())
            .build();
        match r {
            Err(MetaSwarmError::DimensionMismatch { expected: 4, actual: 6 }) => (),
            other => panic!("expected DimensionMismatch(4 vs 6), got {other:?}"),
        }
    }

    #[test]
    fn invalid_coupling_is_rejected() {
        let r = MetaSwarm::builder()
            .id("test")
            .meta_space_dim(4)
            .add_participant(MockParticipant::with_dim("a", 8).boxed())
            .lift_for("a", LinearLift::identity(8, 4).unwrap().boxed())
            .pull_for("a", LinearPull::identity(4, 8).unwrap().boxed())
            .alpha_default(0.7)
            .coupling_gamma(0.5)
            .build();
        match r {
            Err(MetaSwarmError::InvalidCoupling { .. }) => (),
            other => panic!("expected InvalidCoupling, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_participant_id_is_rejected() {
        let r = MetaSwarm::builder()
            .id("test")
            .meta_space_dim(4)
            .add_participant(MockParticipant::with_dim("a", 8).boxed())
            .add_participant(MockParticipant::with_dim("a", 8).boxed())
            .lift_for("a", LinearLift::identity(8, 4).unwrap().boxed())
            .pull_for("a", LinearPull::identity(4, 8).unwrap().boxed())
            .build();
        match r {
            Err(MetaSwarmError::DuplicateParticipantId { id }) => assert_eq!(id, "a"),
            other => panic!("expected DuplicateParticipantId, got {other:?}"),
        }
    }

    // =========================================================================
    // ===  Sprint A1 P1 — PR #3 — Category 12 : Builder proptest (8) =======
    // =========================================================================

    use proptest::prelude::*;

    fn pt_cfg_builder() -> ProptestConfig {
        ProptestConfig { cases: 32, ..ProptestConfig::default() }
    }

    fn make_valid_builder(meta_dim: usize, p_dim: usize) -> MetaSwarmBuilder {
        MetaSwarm::builder()
            .id("pt12")
            .meta_space_dim(meta_dim)
            .add_participant(MockParticipant::with_dim("a", p_dim).boxed())
            .add_participant(MockParticipant::with_dim("b", p_dim).boxed())
            .lift_for("a", LinearLift::identity(p_dim, meta_dim).unwrap().boxed())
            .lift_for("b", LinearLift::identity(p_dim, meta_dim).unwrap().boxed())
            .pull_for("a", LinearPull::identity(meta_dim, p_dim).unwrap().boxed())
            .pull_for("b", LinearPull::identity(meta_dim, p_dim).unwrap().boxed())
            .meta_operator(UniformMetaOp::new(meta_dim).boxed())
            .meta_safety(BoxSafety::symmetric(meta_dim, 5.0).boxed())
    }

    proptest! {
        #![proptest_config(pt_cfg_builder())]

        /// Cat 12 / Test 1 — Random valid configuration builds successfully.
        #[test]
        fn builder_valid_config_pt(
            meta_dim in 2usize..=32,
            p_dim in 2usize..=32,
            alpha in 0.01_f64..0.5,
            gamma in 0.01_f64..0.4,
        ) {
            prop_assume!(alpha + gamma < 0.95);
            let ms = make_valid_builder(meta_dim, p_dim)
                .alpha_default(alpha)
                .coupling_gamma(gamma)
                .build();
            prop_assert!(ms.is_ok());
        }

        /// Cat 12 / Test 2 — Missing meta_space_dim → Internal error.
        #[test]
        fn builder_missing_meta_dim_pt(_filler in 0u8..=255) {
            let r = MetaSwarm::builder()
                .id("pt12-2")
                .add_participant(MockParticipant::with_dim("a", 4).boxed())
                .build();
            let is_internal = matches!(r, Err(MetaSwarmError::Internal(_)));
            prop_assert!(is_internal);
        }

        /// Cat 12 / Test 3 — Empty participants → EmptyMetaSwarm.
        #[test]
        fn builder_empty_participants_pt(_filler in 0u8..=255) {
            let r = MetaSwarm::builder()
                .id("pt12-3")
                .meta_space_dim(4)
                .build();
            let is_empty = matches!(r, Err(MetaSwarmError::EmptyMetaSwarm));
            prop_assert!(is_empty);
        }

        /// Cat 12 / Test 4 — Alpha + gamma ≥ 1 rejected.
        #[test]
        fn builder_invalid_coupling_pt(
            alpha in 0.6_f64..0.95,
            gamma in 0.4_f64..0.95,
        ) {
            prop_assume!(alpha + gamma >= 1.0);
            let r = make_valid_builder(4, 8)
                .alpha_default(alpha)
                .coupling_gamma(gamma)
                .build();
            let is_invalid = matches!(r, Err(MetaSwarmError::InvalidCoupling { .. }));
            prop_assert!(is_invalid);
        }

        /// Cat 12 / Test 5 — Wrong lift dim rejected.
        #[test]
        fn builder_wrong_lift_dim_pt(wrong_out in 1usize..=16) {
            prop_assume!(wrong_out != 4);
            let r = MetaSwarm::builder()
                .id("pt12-5")
                .meta_space_dim(4)
                .add_participant(MockParticipant::with_dim("a", 8).boxed())
                .lift_for("a", LinearLift::identity(8, wrong_out).unwrap().boxed())
                .pull_for("a", LinearPull::identity(4, 8).unwrap().boxed())
                .build();
            let is_dim_err = matches!(r, Err(MetaSwarmError::DimensionMismatch { .. }));
            prop_assert!(is_dim_err);
        }

        /// Cat 12 / Test 6 — Missing pull rejected.
        #[test]
        fn builder_missing_pull_pt(_filler in 0u8..=255) {
            let r = MetaSwarm::builder()
                .id("pt12-6")
                .meta_space_dim(4)
                .add_participant(MockParticipant::with_dim("a", 8).boxed())
                .lift_for("a", LinearLift::identity(8, 4).unwrap().boxed())
                .build();
            let is_missing = matches!(
                r,
                Err(MetaSwarmError::MissingOperator { missing: "pull", .. })
            );
            prop_assert!(is_missing);
        }

        /// Cat 12 / Test 7 — Stray lift for unknown id rejected.
        #[test]
        fn builder_stray_lift_id_pt(_filler in 0u8..=255) {
            let r = MetaSwarm::builder()
                .id("pt12-7")
                .meta_space_dim(4)
                .add_participant(MockParticipant::with_dim("a", 8).boxed())
                .lift_for("a", LinearLift::identity(8, 4).unwrap().boxed())
                .lift_for("unknown", LinearLift::identity(8, 4).unwrap().boxed())
                .pull_for("a", LinearPull::identity(4, 8).unwrap().boxed())
                .build();
            let is_not_found = matches!(
                r,
                Err(MetaSwarmError::ParticipantNotFound { .. })
            );
            prop_assert!(is_not_found);
        }

        /// Cat 12 / Test 8 — Builder setters are idempotent.
        #[test]
        fn builder_setters_idempotent_pt(
            meta_dim in 2usize..=16,
            p_dim in 2usize..=16,
            max_iters in 10usize..=1000,
            tolerance in 1e-6_f64..1e-2,
        ) {
            let ms = make_valid_builder(meta_dim, p_dim)
                .max_iters(max_iters / 2)
                .max_iters(max_iters)
                .tolerance(tolerance * 2.0)
                .tolerance(tolerance)
                .build()
                .unwrap();
            prop_assert_eq!(ms.m(), 2);
        }
    }
}
