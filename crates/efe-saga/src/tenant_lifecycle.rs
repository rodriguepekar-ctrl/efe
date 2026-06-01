//! Sprint Φ-D-OPS (D96) — Tenant lifecycle saga driver.
//!
//! Drives the 12-state FSM declared in
//! `crates/efe-server/migrations/0030_tenant_lifecycle_states.sql`.
//! (Renumbered from `0014_` after Φ-D-OPS post-audit fix B1 ; the
//! original slot collided with `0014_taxonomy_curation.sql`,
//! causing `sqlx::migrate!()` to panic at boot.)
//! The driver wraps the generic `Saga` runner with state transitions
//! gated on the Postgres `tenant_lifecycle_transitions` table.
//!
//! Cf. `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 7.
//!
//! # Invariants
//!
//! 1. Each forward step records its target state **before** executing
//!    its side effect, so a crash mid-step leaves the FSM in the
//!    target state and the bench can replay.
//! 2. Compensation walks back the transition matrix in `compensate`
//!    direction only — it never invents shortcuts.
//! 3. The saga driver is **the only writer** of `lifecycle_state` ;
//!    admin overrides go through `admin_force_state()` which
//!    short-circuits the saga and emits a Sev2 audit row.

use async_trait::async_trait;

use crate::{Saga, SagaContext, SagaError, SagaStep};

/// Strongly-typed handle to one of the 12 lifecycle states.
///
/// Maintained in sync with the SQL `tenant_state` ENUM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LifecycleState {
    SignupIntent,
    ProvisioningPg,
    ProvisioningRedis,
    MigratingSchema,
    SeedingFixtures,
    CompliancePending,
    Active,
    Throttled,
    Suspended,
    ErasurePending,
    ErasureInProgress,
    Erased,
}

impl LifecycleState {
    /// SQL ENUM tag — kept lower-snake.
    #[must_use]
    pub const fn as_sql(self) -> &'static str {
        match self {
            Self::SignupIntent => "signup_intent",
            Self::ProvisioningPg => "provisioning_pg",
            Self::ProvisioningRedis => "provisioning_redis",
            Self::MigratingSchema => "migrating_schema",
            Self::SeedingFixtures => "seeding_fixtures",
            Self::CompliancePending => "compliance_pending",
            Self::Active => "active",
            Self::Throttled => "throttled",
            Self::Suspended => "suspended",
            Self::ErasurePending => "erasure_pending",
            Self::ErasureInProgress => "erasure_in_progress",
            Self::Erased => "erased",
        }
    }

    /// All forward edges of the FSM — same shape as the SQL
    /// `tenant_lifecycle_transitions` table with `direction='forward'`.
    ///
    /// Used by the saga driver to validate a step's `to_state` without
    /// roundtripping to Postgres on each transition.
    #[must_use]
    pub const fn forward_edges() -> &'static [(Self, Self)] {
        use LifecycleState::*;
        &[
            (SignupIntent, ProvisioningPg),
            (ProvisioningPg, ProvisioningRedis),
            (ProvisioningRedis, MigratingSchema),
            (MigratingSchema, SeedingFixtures),
            (SeedingFixtures, CompliancePending),
            (CompliancePending, Active),
            (Active, ErasurePending),
            (Throttled, ErasurePending),
            (Suspended, ErasurePending),
            (ErasurePending, ErasureInProgress),
            (ErasureInProgress, Erased),
        ]
    }

    /// True if the transition `self → next` is on the forward path.
    #[must_use]
    pub fn can_transition_to(self, next: Self) -> bool {
        Self::forward_edges()
            .iter()
            .any(|&(a, b)| a == self && b == next)
    }
}

/// A saga step that drives one lifecycle transition.
pub struct LifecycleStep {
    pub name: &'static str,
    pub from: LifecycleState,
    pub to: LifecycleState,
}

impl LifecycleStep {
    /// Sanity-check the edge at construction time (= debug builds only).
    #[must_use]
    pub fn new(name: &'static str, from: LifecycleState, to: LifecycleState) -> Self {
        debug_assert!(
            from.can_transition_to(to),
            "invalid forward edge {} → {}",
            from.as_sql(),
            to.as_sql()
        );
        Self { name, from, to }
    }
}

#[async_trait]
impl SagaStep for LifecycleStep {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn execute(&self, _ctx: &SagaContext) -> Result<(), SagaError> {
        // Production : UPDATE tenants SET lifecycle_state = self.to.as_sql()
        //              WHERE tenant_id = $1 AND lifecycle_state = self.from.as_sql();
        // The trigger validates + audits ; a 0-row update means the FSM
        // already moved away — we surface that as InvalidState rather
        // than silently winning.
        Ok(())
    }

    async fn compensate(&self, _ctx: &SagaContext) -> Result<(), SagaError> {
        // Production : UPDATE tenants SET lifecycle_state = self.from.as_sql()
        //              WHERE tenant_id = $1 AND lifecycle_state = self.to.as_sql();
        Ok(())
    }
}

/// Builder for the canonical "POC tenant onboarding" saga.
///
/// Cf. `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 7.2.
#[must_use]
pub fn build_onboarding_saga() -> Saga {
    use LifecycleState::*;
    Saga::new("tenant_onboarding")
        .add_step(Box::new(LifecycleStep::new(
            "step_provision_pg", SignupIntent, ProvisioningPg,
        )))
        .add_step(Box::new(LifecycleStep::new(
            "step_provision_redis", ProvisioningPg, ProvisioningRedis,
        )))
        .add_step(Box::new(LifecycleStep::new(
            "step_migrate_schema", ProvisioningRedis, MigratingSchema,
        )))
        .add_step(Box::new(LifecycleStep::new(
            "step_seed_fixtures", MigratingSchema, SeedingFixtures,
        )))
        .add_step(Box::new(LifecycleStep::new(
            "step_request_attestation", SeedingFixtures, CompliancePending,
        )))
        .add_step(Box::new(LifecycleStep::new(
            "step_mark_active", CompliancePending, Active,
        )))
}

/// Builder for the GDPR Art. 17 erasure cascade.
#[must_use]
pub fn build_erasure_saga(starting: LifecycleState) -> Saga {
    use LifecycleState::*;
    // Caller enforces `starting ∈ {Active, Throttled, Suspended}`.
    Saga::new("gdpr_erasure")
        .add_step(Box::new(LifecycleStep::new(
            "step_mark_erasure_pending", starting, ErasurePending,
        )))
        .add_step(Box::new(LifecycleStep::new(
            "step_start_cascade", ErasurePending, ErasureInProgress,
        )))
        .add_step(Box::new(LifecycleStep::new(
            "step_mark_erased", ErasureInProgress, Erased,
        )))
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_edges_have_no_loops() {
        // No (a, a) self-edge.
        for (a, b) in LifecycleState::forward_edges() {
            assert_ne!(a, b, "self-edge on {}", a.as_sql());
        }
    }

    #[test]
    fn signup_to_active_is_reachable_in_six_hops() {
        // BFS shortest path from SignupIntent to Active.
        use std::collections::{HashMap, VecDeque};
        let mut q: VecDeque<(LifecycleState, usize)> = VecDeque::new();
        let mut seen: HashMap<LifecycleState, usize> = HashMap::new();
        q.push_back((LifecycleState::SignupIntent, 0));
        seen.insert(LifecycleState::SignupIntent, 0);
        while let Some((cur, d)) = q.pop_front() {
            if cur == LifecycleState::Active {
                assert_eq!(d, 6, "expected 6-step path, got {d}");
                return;
            }
            for &(a, b) in LifecycleState::forward_edges() {
                if a == cur && !seen.contains_key(&b) {
                    seen.insert(b, d + 1);
                    q.push_back((b, d + 1));
                }
            }
        }
        panic!("Active not reachable from SignupIntent");
    }

    #[test]
    fn erasure_path_three_hops() {
        let s = build_erasure_saga(LifecycleState::Active);
        assert_eq!(s.name, "gdpr_erasure");
        assert_eq!(s.steps.len(), 3);
    }

    #[test]
    fn onboarding_saga_named() {
        let s = build_onboarding_saga();
        assert_eq!(s.name, "tenant_onboarding");
        assert_eq!(s.steps.len(), 6);
    }

    #[test]
    fn invalid_transition_rejected_in_debug() {
        // Active → SeedingFixtures is not a forward edge.
        assert!(!LifecycleState::Active
            .can_transition_to(LifecycleState::SeedingFixtures));
    }

    #[test]
    fn enum_sql_is_lowercase_snake() {
        for s in [
            LifecycleState::SignupIntent,
            LifecycleState::ProvisioningPg,
            LifecycleState::Active,
            LifecycleState::ErasurePending,
            LifecycleState::Erased,
        ] {
            let tag = s.as_sql();
            assert!(tag.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        }
    }
}
