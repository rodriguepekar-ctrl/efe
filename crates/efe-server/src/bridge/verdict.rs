//! Verdict proto ↔ domain conversion.

use crate::proto::v1::Verdict as ProtoVerdict;
use efe_core::Verdict;

/// Convert a domain [`Verdict`] into its proto enum.
#[must_use]
pub fn to_proto(v: &Verdict) -> ProtoVerdict {
    match v {
        Verdict::Continue => ProtoVerdict::Continue,
        Verdict::Converged { .. } => ProtoVerdict::Converged,
        Verdict::Divergence { .. } => ProtoVerdict::Divergence,
        Verdict::ModeCollapse => ProtoVerdict::ModeCollapse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continue_maps() {
        assert_eq!(to_proto(&Verdict::Continue), ProtoVerdict::Continue);
    }

    #[test]
    fn converged_maps() {
        let v = Verdict::Converged { confidence: 0.9 };
        assert_eq!(to_proto(&v), ProtoVerdict::Converged);
    }

    #[test]
    fn divergence_maps() {
        let v = Verdict::Divergence { reason: "explode" };
        assert_eq!(to_proto(&v), ProtoVerdict::Divergence);
    }

    #[test]
    fn mode_collapse_maps() {
        assert_eq!(to_proto(&Verdict::ModeCollapse), ProtoVerdict::ModeCollapse);
    }
}
