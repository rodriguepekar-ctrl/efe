"""Builders for :class:`ConeEvolutionOp` variants."""

from __future__ import annotations

from efe_swarm_integration.types import Anchor, ConeEvolutionOp

__all__ = [
    "append_anchor_op",
    "remove_anchor_op",
    "replace_anchor_op",
    "scale_anchors_op",
    "union_with_key_op",
    "update_radius_op",
]


def append_anchor_op(anchor: Anchor) -> ConeEvolutionOp:
    return ConeEvolutionOp(kind="append_anchor", append_anchor=anchor)


def remove_anchor_op(idx: int) -> ConeEvolutionOp:
    if idx < 0:
        raise ValueError("anchor index must be >= 0")
    return ConeEvolutionOp(kind="remove_anchor_idx", remove_anchor_idx=idx)


def replace_anchor_op(idx: int, anchor: Anchor) -> ConeEvolutionOp:
    if idx < 0:
        raise ValueError("anchor index must be >= 0")
    return ConeEvolutionOp(
        kind="replace_anchor", replace_anchor=(idx, anchor)
    )


def update_radius_op(radius: float) -> ConeEvolutionOp:
    if radius <= 0.0:
        raise ValueError("radius must be > 0")
    return ConeEvolutionOp(kind="update_radius", update_radius=radius)


def scale_anchors_op(factor: float) -> ConeEvolutionOp:
    if factor <= 0.0:
        raise ValueError("scale factor must be > 0")
    return ConeEvolutionOp(kind="scale_anchors", scale_anchors=factor)


def union_with_key_op(target_key: str) -> ConeEvolutionOp:
    if not target_key:
        raise ValueError("target_key is required")
    return ConeEvolutionOp(kind="union_with_key", union_with_key=target_key)
