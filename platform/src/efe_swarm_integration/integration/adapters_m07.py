"""M07 Rollout → M12 preflight adapter.

When M07 considers promoting a new artefact to canary, M12 offers a
**preflight** step that invokes ``SwarmService.DescribeSpec`` on the
server to confirm the HBM + rounds budget *before* spinning any real
inference. This adapter glues M07's ``RolloutClient`` notion of
candidate to M12's :class:`SwarmClient`.

The M07 surface exposes ``RolloutClient``, ``AnalysisResult`` and
registry stores. Here we only absorb :class:`AnalysisResult` — the
type used to report canary observation outcomes — so M12 can package
a ``DescribeSpecReply`` into a canary-compatible analysis.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from efe_rollout import AnalysisResult  # absorbed import

from efe_swarm_integration.client import SwarmClient
from efe_swarm_integration.types import DescribeSpecReply, SwarmSpec

__all__ = ["M07PreflightAdapter", "PreflightVerdict"]


@dataclass(frozen=True, slots=True)
class PreflightVerdict:
    """Preflight outcome ready to embed in an :class:`AnalysisResult`."""

    ok: bool
    hbm_bytes_estimate: int
    rounds_expected: int
    errors: tuple[str, ...]

    def to_analysis_labels(self) -> dict[str, str]:
        return {
            "preflight": "ok" if self.ok else "ko",
            "hbm_bytes_estimate": str(self.hbm_bytes_estimate),
            "rounds_expected": str(self.rounds_expected),
        }


@dataclass(slots=True)
class M07PreflightAdapter:
    """Execute a describe-spec preflight on behalf of M07."""

    swarm: SwarmClient

    async def preflight(
        self,
        *,
        spec: SwarmSpec,
        tenant_id: str,
    ) -> PreflightVerdict:
        try:
            reply: DescribeSpecReply = await self.swarm.describe_spec(
                spec, tenant_id=tenant_id
            )
        except Exception as exc:  # noqa: BLE001
            return PreflightVerdict(
                ok=False,
                hbm_bytes_estimate=0,
                rounds_expected=0,
                errors=(f"describe_spec_failed:{type(exc).__name__}",),
            )
        return PreflightVerdict(
            ok=reply.valid,
            hbm_bytes_estimate=reply.hbm_bytes_estimate,
            rounds_expected=reply.rounds_expected,
            errors=reply.errors,
        )


# Keep the imported symbol visible for typed callers.
_ = AnalysisResult, Any
