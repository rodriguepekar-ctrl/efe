"""Serialise Python :class:`SwarmSpec` / :class:`ExecuteRequest` into the
wire-level dict that the generated ``efe.v1`` protobuf messages will
produce once fed through ``google.protobuf.json_format``.

This module is the **single** place that builds request payloads. It
lives in pure Python : no ``grpcio``/``protobuf`` import required. The
transport layer uses ``ParseDict`` when real gRPC is available, or
``dict → json`` for the :class:`FakeSwarmServer` in tests.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any

from efe_swarm_integration.contract.validator import (
    validate_execute_request,
    validate_swarm_spec,
)
from efe_swarm_integration.errors import SchemaViolation
from efe_swarm_integration.types import (
    AlignmentCone,
    Anchor,
    ConeRef,
    ConvergenceThresholds,
    DecodeMode,
    ExecuteRequest,
    PopulationMix,
    ResourceBudget,
    SwarmSpec,
)

__all__ = ["ContractAdapter", "serialise_execute_request", "serialise_swarm_spec"]


def _anchor_to_dict(a: Anchor) -> dict[str, Any]:
    return {"values": list(a.values)}


def _alignment_cone_to_dict(c: AlignmentCone) -> dict[str, Any]:
    return {
        "anchors": [_anchor_to_dict(a) for a in c.anchors],
        "max_projection_distance": float(c.max_projection_distance),
    }


def _cone_ref_to_dict(r: ConeRef) -> dict[str, Any]:
    return {"tenant": r.tenant, "task": r.task, "version": int(r.version)}


def _pop_to_dict(p: PopulationMix) -> list[dict[str, Any]]:
    return [
        {"template_code": e.template.value, "count": int(e.count)}
        for e in p.entries
    ]


def _conv_to_dict(c: ConvergenceThresholds) -> dict[str, Any]:
    return {
        "epsilon_variance": float(c.epsilon_variance),
        "epsilon_gradient": float(c.epsilon_gradient),
        "epsilon_stability": float(c.epsilon_stability),
        "stability_window": int(c.stability_window),
    }


def _budget_to_dict(b: ResourceBudget) -> dict[str, Any]:
    return {
        "max_wall_clock_ms": int(b.max_wall_clock_ms),
        "max_energy_wh": float(b.max_energy_wh),
        "max_hbm_bytes": int(b.max_hbm_bytes),
        "max_recursion_depth": int(b.max_recursion_depth),
    }


def _decode_mode_to_dict(d: DecodeMode) -> dict[str, Any]:
    if d.kind == "text":
        return {"text": {"max_tokens": int(d.max_tokens or 0)}}
    return {"pass_through": {}}


def serialise_swarm_spec(spec: SwarmSpec) -> dict[str, Any]:
    """Convert a :class:`SwarmSpec` into a protobuf-ready dict.

    The validator is invoked first — callers receive
    :class:`SchemaViolation` on any bound violation.
    """
    validate_swarm_spec(spec)
    out: dict[str, Any] = {
        "swarm_id": spec.swarm_id,
        "problem_embedding": list(spec.problem_embedding),
        "dim_latent": int(spec.dim_latent),
        "population": _pop_to_dict(spec.population),
        "max_rounds": int(spec.max_rounds),
        "convergence": _conv_to_dict(spec.convergence),
        "budget": _budget_to_dict(spec.budget),
    }
    if spec.alignment_cone is not None:
        out["alignment_cone"] = _alignment_cone_to_dict(spec.alignment_cone)
    elif spec.alignment_cone_ref is not None:
        out["alignment_cone_ref"] = _cone_ref_to_dict(spec.alignment_cone_ref)
    else:
        raise SchemaViolation("SwarmSpec requires cone source")  # pragma: no cover
    return out


def serialise_execute_request(req: ExecuteRequest) -> dict[str, Any]:
    """Convert an :class:`ExecuteRequest` into a protobuf-ready dict."""
    validate_execute_request(req)
    out: dict[str, Any] = {
        "request_id": req.request_id,
        "spec": serialise_swarm_spec(req.spec),
        "decode_mode": _decode_mode_to_dict(req.decode_mode),
    }
    if req.raw_prompt is not None:
        out["raw_prompt"] = req.raw_prompt
    if req.deadline is not None:
        out["deadline"] = _datetime_to_proto_ts(req.deadline)
    return out


def _datetime_to_proto_ts(dt: datetime) -> dict[str, Any]:
    """Return the ``google.protobuf.Timestamp`` dict form."""
    if dt.tzinfo is None:
        raise ValueError("timestamp must be tz-aware")
    ts = dt.astimezone(timezone.utc).timestamp()
    seconds = int(ts)
    nanos = int((ts - seconds) * 1_000_000_000)
    return {"seconds": seconds, "nanos": nanos}


# ---------------------------------------------------------------------------
# ContractAdapter — OO facade over the serialisation helpers
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class ContractAdapter:
    """OO façade — the rest of the platform holds one of these."""

    schema_version: str = "efe.v1"

    def build_execute_request(self, req: ExecuteRequest) -> dict[str, Any]:
        return serialise_execute_request(req)

    def build_swarm_spec(self, spec: SwarmSpec) -> dict[str, Any]:
        return serialise_swarm_spec(spec)
