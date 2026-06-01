"""AdminService wire adapters."""

from __future__ import annotations

from datetime import datetime, timezone
from typing import Any

from efe_swarm_integration.contract.adapter_spec import _alignment_cone_to_dict
from efe_swarm_integration.errors import InvalidConeOp, SchemaViolation
from efe_swarm_integration.types import (
    AlignmentCone,
    Anchor,
    CanonicalCone,
    ConeEvolutionOp,
    ConeHandle,
    ConeMetadata,
    ConeSpec,
)

__all__ = [
    "admin_cone_handle_from_dict",
    "admin_evolve_op_to_dict",
    "admin_register_cone_to_dict",
    "parse_canonical_cone",
    "parse_cone_handle",
    "parse_cone_metadata",
]


def admin_register_cone_to_dict(spec: ConeSpec) -> dict[str, Any]:
    return {
        "tenant": spec.tenant,
        "task": spec.task,
        "version": int(spec.version),
        "cone": _alignment_cone_to_dict(spec.cone),
        "created_by": spec.created_by,
        "description": spec.description,
        "labels": {k: v for (k, v) in spec.labels},
    }


def admin_evolve_op_to_dict(op: ConeEvolutionOp) -> dict[str, Any]:
    """Convert an evolution op into the oneof dict form."""
    if op.kind == "append_anchor":
        assert op.append_anchor is not None
        return {"append_anchor": {"values": list(op.append_anchor.values)}}
    if op.kind == "remove_anchor_idx":
        assert op.remove_anchor_idx is not None
        return {"remove_anchor_idx": int(op.remove_anchor_idx)}
    if op.kind == "replace_anchor":
        assert op.replace_anchor is not None
        idx, anchor = op.replace_anchor
        return {
            "replace_anchor": {
                "idx": int(idx),
                "anchor": {"values": list(anchor.values)},
            }
        }
    if op.kind == "update_radius":
        assert op.update_radius is not None
        return {"update_radius": float(op.update_radius)}
    if op.kind == "scale_anchors":
        assert op.scale_anchors is not None
        return {"scale_anchors": float(op.scale_anchors)}
    if op.kind == "union_with_key":
        assert op.union_with_key is not None
        return {"union_with_key": op.union_with_key}
    raise InvalidConeOp(f"unknown op kind: {op.kind}")  # pragma: no cover


def parse_cone_metadata(wire: dict[str, Any]) -> ConeMetadata:
    created_at_raw = wire.get("created_at")
    if isinstance(created_at_raw, dict):
        seconds = int(created_at_raw.get("seconds", 0))
        nanos = int(created_at_raw.get("nanos", 0))
        ts = datetime.fromtimestamp(
            seconds + nanos / 1_000_000_000, tz=timezone.utc
        )
    elif isinstance(created_at_raw, str):
        ts = datetime.fromisoformat(created_at_raw.replace("Z", "+00:00"))
    else:
        ts = datetime.now(tz=timezone.utc)
    labels_raw = wire.get("labels", {}) or {}
    if isinstance(labels_raw, dict):
        labels = tuple(sorted(labels_raw.items()))
    else:
        labels = ()
    return ConeMetadata(
        created_at=ts,
        created_by=str(wire.get("created_by", "")),
        description=str(wire.get("description", "")),
        parent_key=wire.get("parent_key") or None,
        evolution_op=wire.get("evolution_op") or None,
        labels=labels,
    )


def _parse_alignment_cone(wire: dict[str, Any]) -> AlignmentCone:
    anchors_wire = wire.get("anchors") or []
    anchors = tuple(
        Anchor(values=tuple(float(x) for x in (a.get("values") or [])))
        for a in anchors_wire
    )
    return AlignmentCone(
        anchors=anchors,
        max_projection_distance=float(wire.get("max_projection_distance", 0.0)),
    )


def parse_canonical_cone(wire: dict[str, Any]) -> CanonicalCone:
    cone = wire.get("cone")
    if not isinstance(cone, dict):
        raise SchemaViolation("CanonicalCone missing cone")
    return CanonicalCone(
        cone=_parse_alignment_cone(cone),
        metadata=parse_cone_metadata(wire.get("metadata") or {}),
    )


def parse_cone_handle(wire: dict[str, Any]) -> ConeHandle:
    return ConeHandle(
        key=str(wire.get("key", "")),
        canonical=parse_canonical_cone(wire.get("cone") or {}),
    )


def admin_cone_handle_from_dict(wire: dict[str, Any]) -> ConeHandle:
    """Public alias for callers outside the module."""
    return parse_cone_handle(wire)
