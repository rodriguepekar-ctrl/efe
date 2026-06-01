//! Post-collapse invariant checks.
//!
//! After the engine selects a recoverable outcome, the caller may run
//! [`validate_post_collapse`] to double-check the ELS is truly in a
//! sane state. The checks are cheap (one `compute_stats` call) and
//! detect residual corruption.

use crate::collapse::config::CollapseConfig;
use crate::els::{compute_stats, EpistemicLatentSpace};
use crate::errors::Result;
use crate::memory::Arena;

/// Result of a post-collapse validation.
#[derive(Clone, Debug, PartialEq)]
#[must_use]
pub enum PostValidation {
    /// ELS is in a clean state.
    Ok {
        /// Variance observed.
        variance: f32,
        /// Active agent count.
        n_active: u32,
    },
    /// ELS still carries NaN / Inf.
    StillAnomalous,
    /// ELS variance exceeds `config.post_variance_max`.
    VarianceTooHigh {
        /// Actual variance observed.
        variance: f32,
        /// Configured maximum.
        maximum: f32,
    },
    /// ELS has no active agents (fully deactivated by a respawn).
    NoActiveAgents,
}

impl PostValidation {
    /// True iff the ELS is in a clean state.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok { .. })
    }
}

/// Validate the ELS post-collapse.
///
/// # Errors
///
/// Propagates [`compute_stats`] failures (e.g. ELS stub).
pub fn validate_post_collapse(
    els: &EpistemicLatentSpace,
    arena: &Arena,
    config: &CollapseConfig,
) -> Result<PostValidation> {
    let stats = compute_stats(els, arena)?;
    if !stats.all_finite() {
        return Ok(PostValidation::StillAnomalous);
    }
    if stats.n_active == 0 {
        return Ok(PostValidation::NoActiveAgents);
    }
    if stats.variance > config.post_variance_max {
        return Ok(PostValidation::VarianceTooHigh {
            variance: stats.variance,
            maximum: config.post_variance_max,
        });
    }
    Ok(PostValidation::Ok {
        variance: stats.variance,
        n_active: stats.n_active,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_validation_ok_is_ok() {
        let v = PostValidation::Ok {
            variance: 0.01,
            n_active: 32,
        };
        assert!(v.is_ok());
    }

    #[test]
    fn post_validation_others_not_ok() {
        assert!(!PostValidation::StillAnomalous.is_ok());
        assert!(!PostValidation::NoActiveAgents.is_ok());
        assert!(!PostValidation::VarianceTooHigh {
            variance: 100.0,
            maximum: 10.0,
        }
        .is_ok());
    }
}
