// SPDX-License-Identifier: UNLICENSED
//! Erreurs Phase 3.

use efe_coordinator::CoordError;

/// Résultat shorthand Phase 3.
pub type MultiLlmResult<T> = std::result::Result<T, MultiLlmError>;

/// Erreurs Phase 3 — encapsule + étend `CoordError`.
#[derive(Debug, thiserror::Error)]
pub enum MultiLlmError {
    /// Encapsulation d'une erreur Coordinator.
    #[error("coordinator: {0}")]
    Coord(#[from] CoordError),

    /// Le budget hard-cap a été dépassé (invariant I13).
    #[error("budget exceeded: spent={spent_cents} / max={max_cents}")]
    BudgetExceeded {
        /// Total déjà engagé en cents.
        spent_cents: u32,
        /// Plafond en cents.
        max_cents: u32,
    },

    /// Un workflow Debate a échoué : pas assez de LLMs (besoin ≥ 3).
    #[error("debate requires ≥ 3 LLMs (2 débateurs + 1 jury), got {n}")]
    DebateTooFewLlms {
        /// Nombre fourni.
        n: usize,
    },

    /// Un workflow Relay a échoué : un LLM down et pas de fallback.
    #[error("relay phase '{phase}' failed: {reason}")]
    RelayPhaseFailed {
        /// Phase α/β/γ.
        phase: String,
        /// Cause.
        reason: String,
    },

    /// Specialist : decomposer a produit > `SPECIALIST_MAX_SUB_TASKS`.
    #[error("decomposer produced too many sub-tasks: {n} > {max}")]
    TooManySubTasks {
        /// Sub-tasks produites.
        n: usize,
        /// Borne dure.
        max: usize,
    },

    /// Specialist : la contrainte anti-monoculture (I11) a été violée
    /// et même après re-routing il reste un déséquilibre.
    #[error("anti-monoculture constraint cannot be satisfied: {dominant_llm} would own {} of {total} sub-tasks (limit {})", format_pct(*ratio), format_pct(*limit))]
    MonocultureViolation {
        /// LLM dominant.
        dominant_llm: String,
        /// Ratio observé 0..1.
        ratio: f32,
        /// Limite (~0.60).
        limit: f32,
        /// Total sub-tasks.
        total: usize,
    },

    /// Cascade : tous les niveaux (drafter + verifier + validator) ont
    /// échoué.
    #[error("cascade exhausted all levels: drafter={drafter_err}; verifier={verifier_err}; validator={validator_err}")]
    CascadeExhausted {
        /// Erreur drafter (None si succès).
        drafter_err: String,
        /// Erreur verifier.
        verifier_err: String,
        /// Erreur validator.
        validator_err: String,
    },

    /// Le `Reformulator` a échoué à parser le dialecte source.
    #[error("reformulation failed from {from_dialect} to {to_schema}: {reason}")]
    ReformulationFailed {
        /// Dialecte source (`CLAUDE_XML` | `GPT4_JSON` | `GEMINI_MARKDOWN`).
        from_dialect: String,
        /// Schema cible (`canonical/plan_v1` etc).
        to_schema: String,
        /// Cause.
        reason: String,
    },

    /// Validation JSON Schema d'un canonical output a échoué.
    #[error("canonical validation failed for {schema}: {error}")]
    CanonicalValidation {
        /// Schema.
        schema: String,
        /// Erreur de validation.
        error: String,
    },

    /// Tous les LLMs sont down (chaos scénario C).
    #[error("all LLMs are down — no workflow can run")]
    AllLlmsDown,

    /// Tentative d'accès cross-tenant (invariant I12).
    #[error("cross-tenant access attempted: tenant {actor} tried to read tenant {target}")]
    CrossTenantAccess {
        /// Tenant qui a tenté.
        actor: String,
        /// Tenant cible.
        target: String,
    },

    /// Erreur de configuration au boot.
    #[error("config error: {0}")]
    Config(String),
}

fn format_pct(ratio: f32) -> String {
    format!("{:.0}%", ratio * 100.0)
}

impl MultiLlmError {
    /// L'erreur est-elle réessayable au niveau workflow ?
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Coord(c) => c.is_retryable(),
            Self::BudgetExceeded { .. } => false,
            Self::DebateTooFewLlms { .. } => false,
            Self::RelayPhaseFailed { .. } => true,
            Self::TooManySubTasks { .. } => false,
            Self::MonocultureViolation { .. } => false,
            Self::CascadeExhausted { .. } => true,
            Self::ReformulationFailed { .. } => false,
            Self::CanonicalValidation { .. } => false,
            Self::AllLlmsDown => true,
            Self::CrossTenantAccess { .. } => false,
            Self::Config(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_exceeded_display() {
        let e = MultiLlmError::BudgetExceeded {
            spent_cents: 1500,
            max_cents: 1000,
        };
        let s = format!("{e}");
        assert!(s.contains("1500"));
        assert!(s.contains("1000"));
        assert!(!e.is_retryable());
    }

    #[test]
    fn debate_too_few_not_retryable() {
        let e = MultiLlmError::DebateTooFewLlms { n: 2 };
        assert!(!e.is_retryable());
    }

    #[test]
    fn all_llms_down_is_retryable() {
        assert!(MultiLlmError::AllLlmsDown.is_retryable());
    }

    #[test]
    fn cross_tenant_never_retryable() {
        let e = MultiLlmError::CrossTenantAccess {
            actor: "tenant-a".into(),
            target: "tenant-b".into(),
        };
        assert!(!e.is_retryable());
        assert!(format!("{e}").contains("tenant-a"));
    }

    #[test]
    fn monoculture_message_includes_ratio() {
        let e = MultiLlmError::MonocultureViolation {
            dominant_llm: "claude".into(),
            ratio: 0.80,
            limit: 0.60,
            total: 10,
        };
        let s = format!("{e}");
        assert!(s.contains("claude"));
        assert!(s.contains("80%"));
    }

    #[test]
    fn from_coord_error_works() {
        let c = CoordError::Config("x".into());
        let m: MultiLlmError = c.into();
        assert!(matches!(m, MultiLlmError::Coord(_)));
    }
}
