//! `AlignmentCone` and M9 governance types ↔ proto conversion.

use std::time::{SystemTime, UNIX_EPOCH};

use prost_types::Timestamp;

use crate::proto::v1 as proto;
use crate::proto::v1_admin as admin;
use efe_core::alignment::{
    BreachContext, BreachReport, CanonicalCone, ConeEvolutionOp, ConeHandle, ConeKey, ConeMetadata,
};
use efe_core::{AlignmentCone, EfeError, Result};

/// Convert a proto `AlignmentCone` into the domain form.
pub fn from_proto(p: proto::AlignmentCone) -> Result<AlignmentCone> {
    let anchors: Vec<Vec<f32>> = p.anchors.into_iter().map(|a| a.values).collect();
    AlignmentCone::new(anchors, p.max_projection_distance)
}

/// Convert a domain `AlignmentCone` back to proto.
#[must_use]
pub fn to_proto(c: &AlignmentCone) -> proto::AlignmentCone {
    proto::AlignmentCone {
        anchors: c
            .anchors
            .iter()
            .map(|a| proto::Anchor { values: a.clone() })
            .collect(),
        max_projection_distance: c.max_projection_distance,
    }
}

/// Build an admin proto `ConeHandle` from a domain [`ConeHandle`].
#[must_use]
pub fn handle_to_proto(h: ConeHandle) -> admin::ConeHandle {
    let key = h.key.to_string();
    admin::ConeHandle {
        key,
        cone: Some(canonical_to_proto(&h.cone)),
    }
}

/// Build an admin `CanonicalCone` from the domain type.
#[must_use]
pub fn canonical_to_proto(c: &CanonicalCone) -> admin::CanonicalCone {
    admin::CanonicalCone {
        cone: Some(to_proto(&c.cone)),
        metadata: Some(metadata_to_proto(&c.metadata)),
    }
}

/// Convert [`ConeMetadata`] → proto.
#[must_use]
pub fn metadata_to_proto(m: &ConeMetadata) -> admin::ConeMetadata {
    let mut labels = std::collections::HashMap::new();
    for (k, v) in &m.labels {
        labels.insert(k.clone(), v.clone());
    }
    admin::ConeMetadata {
        created_at: Some(nanos_to_timestamp(m.created_at_nanos)),
        created_by: m.created_by.clone(),
        description: m.description.clone(),
        parent_key: m
            .parent
            .as_ref()
            .map(ConeKey::to_string)
            .unwrap_or_default(),
        evolution_op: m
            .evolution_op
            .as_ref()
            .map(|op| op.kind().to_string())
            .unwrap_or_default(),
        labels,
    }
}

/// Build a [`ConeEvolutionOp`] from its proto form.
pub fn evolution_op_from_proto(p: admin::ConeEvolutionOp) -> Result<ConeEvolutionOp> {
    use admin::cone_evolution_op::Op;
    let Some(op) = p.op else {
        return Err(EfeError::InvalidConfig(
            "ConeEvolutionOp: missing op variant".into(),
        ));
    };
    Ok(match op {
        Op::AppendAnchor(a) => ConeEvolutionOp::AppendAnchor { anchor: a.values },
        Op::RemoveAnchorIdx(idx) => ConeEvolutionOp::RemoveAnchor { idx: idx as usize },
        Op::ReplaceAnchor(r) => ConeEvolutionOp::ReplaceAnchor {
            idx: r.idx as usize,
            anchor: r.anchor.map(|a| a.values).unwrap_or_default(),
        },
        Op::UpdateRadius(r) => ConeEvolutionOp::UpdateRadius { new_radius: r },
        Op::ScaleAnchors(f) => ConeEvolutionOp::ScaleAnchors { factor: f },
        Op::UnionWithKey(k) => ConeEvolutionOp::UnionWith {
            other: ConeKey::parse(&k)?,
        },
    })
}

/// Convert a [`BreachReport`] into its admin proto form.
#[must_use]
pub fn breach_to_proto(r: BreachReport) -> admin::BreachReport {
    let ctx = match &r.context {
        BreachContext::Dissolution => "dissolution".to_string(),
        BreachContext::FusionRound { round } => format!("fusion@round{round}"),
        BreachContext::Manual { reason } => format!("manual: {reason}"),
        // Fallback for future non_exhaustive variants.
        _ => "unknown".to_string(),
    };
    admin::BreachReport {
        cone_key: r.cone_key.to_string(),
        swarm_id: r.swarm_id.as_uuid().to_string(),
        observed_delta: r.observed_delta,
        cone_radius: r.cone_radius,
        timestamp: Some(nanos_to_timestamp(r.timestamp_nanos)),
        context: ctx,
    }
}

fn nanos_to_timestamp(nanos: u64) -> Timestamp {
    let secs = (nanos / 1_000_000_000) as i64;
    let nanos_part = (nanos % 1_000_000_000) as i32;
    Timestamp {
        seconds: secs,
        nanos: nanos_part,
    }
}

/// Return the current wall-clock as a `Timestamp` (helper).
#[must_use]
pub fn now_timestamp() -> Timestamp {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp {
        seconds: now.as_secs() as i64,
        nanos: now.subsec_nanos() as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cone() -> AlignmentCone {
        AlignmentCone::new(vec![vec![1.0, 0.0], vec![0.0, 1.0]], 0.5).unwrap()
    }

    #[test]
    fn cone_roundtrip() {
        let p = to_proto(&cone());
        let back = from_proto(p).unwrap();
        assert_eq!(back.anchors.len(), 2);
        assert!((back.max_projection_distance - 0.5).abs() < 1e-6);
    }

    #[test]
    fn cone_empty_anchors_error() {
        let p = proto::AlignmentCone {
            anchors: vec![],
            max_projection_distance: 0.5,
        };
        assert!(from_proto(p).is_err());
    }

    #[test]
    fn metadata_roundtrip_has_parent_key_empty_for_root() {
        let md = ConeMetadata::new("bot", "initial").unwrap();
        let p = metadata_to_proto(&md);
        assert_eq!(p.parent_key, "");
        assert_eq!(p.evolution_op, "");
        assert_eq!(p.created_by, "bot");
    }

    #[test]
    fn evolution_op_append_roundtrip() {
        let proto_op = admin::ConeEvolutionOp {
            op: Some(admin::cone_evolution_op::Op::AppendAnchor(proto::Anchor {
                values: vec![0.5, 0.5],
            })),
        };
        let domain = evolution_op_from_proto(proto_op).unwrap();
        match domain {
            ConeEvolutionOp::AppendAnchor { anchor } => {
                assert_eq!(anchor, vec![0.5, 0.5]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn evolution_op_union_with_key_parses() {
        let proto_op = admin::ConeEvolutionOp {
            op: Some(admin::cone_evolution_op::Op::UnionWithKey(
                "acme/chat-qa@v3".into(),
            )),
        };
        let domain = evolution_op_from_proto(proto_op).unwrap();
        match domain {
            ConeEvolutionOp::UnionWith { other } => {
                assert_eq!(other.tenant, "acme");
                assert_eq!(other.task, "chat-qa");
                assert_eq!(other.version, 3);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn evolution_op_missing_inner_op_errors() {
        let proto_op = admin::ConeEvolutionOp { op: None };
        assert!(evolution_op_from_proto(proto_op).is_err());
    }

    #[test]
    fn breach_context_dissolution_renders() {
        use efe_core::SwarmId;
        let r = BreachReport::new(
            ConeKey::new("t", "a", 1).unwrap(),
            SwarmId::new(),
            0.9,
            0.5,
            1_000_000_000,
            BreachContext::Dissolution,
        );
        let p = breach_to_proto(r);
        assert_eq!(p.context, "dissolution");
    }

    #[test]
    fn breach_context_fusion_round_renders() {
        use efe_core::SwarmId;
        let r = BreachReport::new(
            ConeKey::new("t", "a", 1).unwrap(),
            SwarmId::new(),
            0.9,
            0.5,
            1_000_000_000,
            BreachContext::FusionRound { round: 3 },
        );
        assert_eq!(breach_to_proto(r).context, "fusion@round3");
    }
}
