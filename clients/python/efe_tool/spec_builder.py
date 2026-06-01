"""Build a :class:`efe.v1.SwarmSpec` proto from a catalog + problem vector.

Keeping this in its own module makes it trivial to unit-test the proto
shape without round-tripping through gRPC.
"""

from __future__ import annotations

from typing import Any

import numpy as np

from .catalog import AnchorCatalog


DEFAULT_POPULATION = [("na-sem", 1)]  # will be blown up by the server's heuristic
DEFAULT_MAX_ROUNDS = 16

# The server's `ResourceBudget::default().max_wall_clock_ms = 500` is too
# tight for a 1024-agent CPU swarm (typical MVP config). We override it
# from the client to avoid spurious `ResourceExhausted`. 5 s is well
# above the p99 we measure on the reference workstation; scale down to
# 1-2 s when moving to GPU.
DEFAULT_BUDGET_WALL_CLOCK_MS = 5_000
DEFAULT_BUDGET_HBM_BYTES = 6 * 1024 * 1024 * 1024  # 6 GiB
DEFAULT_BUDGET_ENERGY_WH = 5.0


def build_execute_request(
    *,
    request_id: str,
    text: str,
    problem_embedding: np.ndarray,
    catalog: AnchorCatalog,
    max_rounds: int = DEFAULT_MAX_ROUNDS,
    budget_wall_clock_ms: int = DEFAULT_BUDGET_WALL_CLOCK_MS,
    include_raw_prompt: bool = False,
) -> Any:
    """Build an ``ExecuteRequest`` proto ready for ``SwarmService.Execute``.

    Args:
        request_id: Idempotency + audit key.
        text: Original user text. Included in the proto ONLY when
            ``include_raw_prompt=True`` (default False ; the audit
            policy forbids leaking user text).
        problem_embedding: The embedding to route. Must be a
            ``np.ndarray`` of shape ``(catalog.dim,)``.
        catalog: Loaded :class:`AnchorCatalog`.
        max_rounds: Upper bound on fusion rounds.
        include_raw_prompt: Override the no-PII default. Reserved for
            dev environments.

    Returns:
        An ``efe.v1.ExecuteRequest`` message.

    Raises:
        ValueError: dim mismatch.
    """
    from efe_tool._proto.efe.v1 import efe_pb2, types_pb2  # lazy import

    if problem_embedding.shape != (catalog.dim,):
        raise ValueError(
            f"problem_embedding shape {problem_embedding.shape} != (catalog.dim={catalog.dim},)"
        )
    if not np.isfinite(problem_embedding).all():
        raise ValueError("problem_embedding contains non-finite values")

    # Population: we let the server's heuristic estimator decide. We
    # provide one non-empty entry referencing the MVP template
    # "na-sem" so the spec validates; the genesis engine will scale
    # up to its configured min_agents / max_agents bracket.
    population = [
        types_pb2.PopulationEntry(template_code=code, count=count)
        for code, count in DEFAULT_POPULATION
    ]

    # Alignment cone with the catalog's anchors.
    anchors_proto = [
        types_pb2.Anchor(values=catalog.anchors[i].astype("float32").tolist())
        for i in range(catalog.anchors.shape[0])
    ]
    cone = types_pb2.AlignmentCone(
        anchors=anchors_proto,
        max_projection_distance=float(catalog.max_projection_distance),
    )

    budget = types_pb2.ResourceBudget(
        max_wall_clock_ms=int(budget_wall_clock_ms),
        max_energy_wh=float(DEFAULT_BUDGET_ENERGY_WH),
        max_hbm_bytes=int(DEFAULT_BUDGET_HBM_BYTES),
        max_recursion_depth=0,
    )

    spec = types_pb2.SwarmSpec(
        dim_latent=int(catalog.dim),
        problem_embedding=problem_embedding.astype("float32").tolist(),
        population=population,
        max_rounds=int(max_rounds),
        alignment_cone=cone,
        budget=budget,
    )

    decode_mode = types_pb2.DecodeMode(pass_through=types_pb2.PassThroughMode())

    return efe_pb2.ExecuteRequest(
        request_id=request_id,
        raw_prompt=text if include_raw_prompt else "",
        spec=spec,
        decode_mode=decode_mode,
    )
