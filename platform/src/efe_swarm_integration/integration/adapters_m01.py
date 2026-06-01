"""M01 Platform → M12 legacy bridge.

``efe_platform`` ships ``Bot`` + ``SwarmClient`` + ``SwarmSpec`` built
before M12 existed. This adapter reuses the M01 ``SwarmSpec`` shape as
an **input** surface : callers with M01 code still in flight can feed
M01 specs into M12 :meth:`describe_spec` / :meth:`execute` without a
rewrite.

The bridge is a thin *adapter*, not a compatibility shim — it
converts an M01 ``SwarmSpec`` value to the M12 dataclass and delegates.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from efe_platform import SwarmSpec as M01SwarmSpec  # absorbed import

from efe_swarm_integration.client import SwarmClient
from efe_swarm_integration.types import (
    AlignmentCone,
    Anchor,
    ConeRef,
    ExecuteRequest,
    PopulationEntry,
    PopulationMix,
    SwarmSpec as M12SwarmSpec,
    TemplateCode,
)

__all__ = ["M01PlatformAdapter", "m01_spec_to_m12"]


def m01_spec_to_m12(spec: M01SwarmSpec, *, cone_ref: ConeRef | None = None) -> M12SwarmSpec:
    """Convert an ``efe_platform.SwarmSpec`` into the M12 dataclass.

    Only the fields shared by the two APIs survive the translation ;
    the caller supplies a :class:`ConeRef` or falls back to an empty
    inline cone (tests only).
    """
    # M01 SwarmSpec is a simple dataclass ; we probe attributes
    # defensively because the two lineages evolved separately.
    dim = int(getattr(spec, "dim_latent", 0) or len(getattr(spec, "problem_embedding", ())))
    embedding = tuple(
        float(x) for x in getattr(spec, "problem_embedding", ())
    )
    population_entries = getattr(spec, "population", ())
    if isinstance(population_entries, PopulationMix):
        pop = population_entries
    else:
        entries: list[PopulationEntry] = []
        for ent in population_entries:
            # tuple form (template_code, count) OR dataclass
            if isinstance(ent, tuple) and len(ent) == 2:
                code, count = ent
                if isinstance(code, str):
                    tc = TemplateCode(code)
                elif isinstance(code, TemplateCode):
                    tc = code
                else:
                    tc = TemplateCode.NaSem
                entries.append(PopulationEntry(tc, int(count)))
        pop = (
            PopulationMix(entries=tuple(entries))
            if entries
            else PopulationMix.default_classifier()
        )

    if cone_ref is None:
        cone = AlignmentCone(
            anchors=(Anchor(values=tuple([0.0] * dim)),),
            max_projection_distance=0.35,
        )
        return M12SwarmSpec(
            dim_latent=dim,
            problem_embedding=embedding,
            population=pop,
            alignment_cone=cone,
            alignment_cone_ref=None,
        )
    return M12SwarmSpec(
        dim_latent=dim,
        problem_embedding=embedding,
        population=pop,
        alignment_cone=None,
        alignment_cone_ref=cone_ref,
    )


@dataclass(slots=True)
class M01PlatformAdapter:
    """Bridge so M01 callers can keep their ``SwarmSpec`` and still go
    through M12 for flow-control, HBM budget, audit, etc."""

    swarm: SwarmClient

    async def describe(
        self,
        spec: M01SwarmSpec,
        *,
        tenant_id: str,
        cone_ref: ConeRef | None = None,
    ) -> Any:
        m12_spec = m01_spec_to_m12(spec, cone_ref=cone_ref)
        return await self.swarm.describe_spec(m12_spec, tenant_id=tenant_id)


_ = ExecuteRequest  # keep import live
