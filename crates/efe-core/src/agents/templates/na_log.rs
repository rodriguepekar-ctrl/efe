//! **Sprint D33** — Logical-inference nano-agent template (MVP).
//!
//! ## Rôle
//!
//! NaLog handles **logical inference** queries — those where the
//! correct answer requires composing two or more premises into a
//! conclusion. Its kernel applies a small symbolic-reasoning
//! transformation to the latent representation before fusion.
//!
//! Default share 5 % in a balanced GA mix (small but stable
//! contributor). MVP-era default consensus weight 1.0.

use super::{FusionKernelId, NanoAgentTemplate};
use crate::types::TemplateCode;

/// Logical-inference template.
///
/// MVP-era. Specialises in queries with conditional structure
/// (if/then, all/some, modal qualifiers). Its consensus vote is
/// most informative when the input phrase contains explicit logical
/// markers detected at genesis time.
#[derive(Debug, Clone, Copy, Default)]
pub struct NaLog;

impl NanoAgentTemplate for NaLog {
    fn code(&self) -> TemplateCode {
        TemplateCode::NaLog
    }

    fn default_share(&self) -> f32 {
        // 5 % — small but stable contributor.
        0.05
    }

    fn fusion_kernel(&self) -> FusionKernelId {
        FusionKernelId::Logical
    }

    fn consensus_weight(&self) -> f32 {
        1.0
    }

    fn description(&self) -> &'static str {
        "Logical-inference template — conditional / modal reasoning"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn na_log_has_correct_code() {
        assert_eq!(NaLog.code(), TemplateCode::NaLog);
        assert_eq!(NaLog.code().as_u8(), 1);
        assert_eq!(NaLog.code().as_str(), "na-log");
    }

    #[test]
    fn na_log_share_is_small() {
        assert_eq!(NaLog.default_share(), 0.05);
    }

    #[test]
    fn na_log_kernel_is_logical() {
        assert_eq!(NaLog.fusion_kernel(), FusionKernelId::Logical);
    }

    #[test]
    fn na_log_is_mvp() {
        assert!(NaLog.code().is_mvp());
    }
}
