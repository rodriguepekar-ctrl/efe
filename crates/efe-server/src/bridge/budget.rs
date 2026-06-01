//! `ResourceBudget` ↔ proto conversion.

use crate::proto::v1 as proto;
use efe_core::{EfeError, ResourceBudget, Result};

/// Build a domain `ResourceBudget` from its proto form.
///
/// `ResourceBudget` is `#[non_exhaustive]` cross-crate, so we derive
/// from `Default` and assign each public field explicitly.
pub fn from_proto(p: proto::ResourceBudget) -> Result<ResourceBudget> {
    let max_recursion: u8 = p.max_recursion_depth.try_into().map_err(|_| {
        EfeError::InvalidConfig(format!(
            "max_recursion_depth {} does not fit in u8",
            p.max_recursion_depth
        ))
    })?;
    let mut b = ResourceBudget::default();
    b.max_wall_clock_ms = p.max_wall_clock_ms;
    b.max_energy_wh = p.max_energy_wh;
    b.max_hbm_bytes = p.max_hbm_bytes;
    b.max_recursion_depth = max_recursion;
    Ok(b)
}

/// Convert a domain `ResourceBudget` back into its proto form.
#[must_use]
pub fn to_proto(b: ResourceBudget) -> proto::ResourceBudget {
    proto::ResourceBudget {
        max_wall_clock_ms: b.max_wall_clock_ms,
        max_energy_wh: b.max_energy_wh,
        max_hbm_bytes: b.max_hbm_bytes,
        max_recursion_depth: u32::from(b.max_recursion_depth),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_default() {
        let domain = ResourceBudget::default();
        let p = to_proto(domain);
        let back = from_proto(p).unwrap();
        assert_eq!(domain.max_wall_clock_ms, back.max_wall_clock_ms);
        assert_eq!(domain.max_hbm_bytes, back.max_hbm_bytes);
        assert_eq!(domain.max_recursion_depth, back.max_recursion_depth);
    }

    #[test]
    fn rejects_overflow_recursion_depth() {
        let p = proto::ResourceBudget {
            max_recursion_depth: 1000,
            ..Default::default()
        };
        assert!(from_proto(p).is_err());
    }
}
