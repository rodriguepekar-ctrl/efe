"""Ex-ante HBM estimation."""

from __future__ import annotations

from dataclasses import dataclass

from efe_swarm_integration.types import SwarmSpec

__all__ = ["HbmEstimator"]


@dataclass(slots=True)
class HbmEstimator:
    """Produce a conservative upper bound on the HBM a swarm will use.

    Formula (matches architecture §14.2) :

        hbm_bytes ≈ 4 · dim · (population + rounds · buffer_factor)
                   + 4 · dim · anchors · 2
                   + overhead_static
    """

    buffer_factor: float = 1.5
    overhead_static_bytes: int = 2 * 1024 * 1024  # 2 MiB

    def estimate(self, spec: SwarmSpec, *, anchors_count: int = 0) -> int:
        dim = spec.dim_latent
        pop = spec.population.total
        rounds = spec.max_rounds
        anchors = anchors_count
        if spec.alignment_cone is not None:
            anchors = max(anchors, len(spec.alignment_cone.anchors))

        core = 4 * dim * (pop + int(rounds * self.buffer_factor))
        anchor_cost = 4 * dim * anchors * 2
        return core + anchor_cost + self.overhead_static_bytes
