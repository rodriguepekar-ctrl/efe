"""Validation at the wire frontier.

Every ``ContractAdapter.build_*`` method passes through the validator
here BEFORE producing any wire-level payload. This keeps the rest of
the module (and the Rust server) free of malformed inputs.
"""

from __future__ import annotations

from efe_swarm_integration.config import (
    MAX_ANCHORS,
    MAX_CLIENT_DEADLINE_S,
    MAX_DIM,
    MAX_HBM_BYTES,
    MAX_POPULATION,
    MAX_ROUNDS,
)
from efe_swarm_integration.errors import SchemaViolation
from efe_swarm_integration.types import (
    AlignmentCone,
    ExecuteRequest,
    SwarmSpec,
)

__all__ = [
    "validate_alignment_cone",
    "validate_execute_request",
    "validate_swarm_spec",
]


def validate_swarm_spec(spec: SwarmSpec) -> None:
    """Raise :class:`SchemaViolation` on any bound violation."""
    if spec.dim_latent < 1 or spec.dim_latent > MAX_DIM:
        raise SchemaViolation(
            f"dim_latent out of range [1, {MAX_DIM}]",
            detail={"dim_latent": spec.dim_latent},
        )
    if len(spec.problem_embedding) != spec.dim_latent:
        raise SchemaViolation(
            "problem_embedding length mismatch",
            detail={
                "expected": spec.dim_latent,
                "got": len(spec.problem_embedding),
            },
        )
    if spec.population.total > MAX_POPULATION:
        raise SchemaViolation(
            f"population.total exceeds MAX_POPULATION={MAX_POPULATION}",
            detail={"total": spec.population.total},
        )
    if spec.max_rounds > MAX_ROUNDS:
        raise SchemaViolation(
            f"max_rounds exceeds MAX_ROUNDS={MAX_ROUNDS}",
            detail={"max_rounds": spec.max_rounds},
        )
    if spec.budget.max_hbm_bytes > MAX_HBM_BYTES:
        raise SchemaViolation(
            f"budget.max_hbm_bytes exceeds MAX_HBM_BYTES",
            detail={"max_hbm_bytes": spec.budget.max_hbm_bytes, "cap": MAX_HBM_BYTES},
        )
    if spec.budget.max_wall_clock_ms > MAX_CLIENT_DEADLINE_S * 1000:
        raise SchemaViolation(
            f"budget.max_wall_clock_ms exceeds MAX_CLIENT_DEADLINE",
            detail={"ms": spec.budget.max_wall_clock_ms},
        )
    if spec.alignment_cone is not None:
        validate_alignment_cone(spec.alignment_cone, expected_dim=spec.dim_latent)
    # When using ConeRef, dimension is verified server-side at lookup.


def validate_alignment_cone(cone: AlignmentCone, *, expected_dim: int) -> None:
    if len(cone.anchors) > MAX_ANCHORS:
        raise SchemaViolation(
            f"anchors count exceeds MAX_ANCHORS={MAX_ANCHORS}",
            detail={"count": len(cone.anchors)},
        )
    if cone.dim() != expected_dim:
        raise SchemaViolation(
            "anchor dim mismatches spec.dim_latent",
            detail={"cone_dim": cone.dim(), "spec_dim": expected_dim},
        )


def validate_execute_request(req: ExecuteRequest) -> None:
    validate_swarm_spec(req.spec)
    if req.raw_prompt is not None and len(req.raw_prompt) > 200_000:
        raise SchemaViolation(
            "raw_prompt exceeds 200 kB",
            detail={"len": len(req.raw_prompt)},
        )
