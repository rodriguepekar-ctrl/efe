"""M06 Inference → M12 routing adapter.

M06 owns inference traffic ; when a prompt classification is needed,
the M06 inference adapter calls the M12 :class:`SwarmClient`. This
module absorbs M06's :class:`InferenceSettings` so the caller can
derive the correct tenant / task / budget triple.
"""

from __future__ import annotations

from dataclasses import dataclass

from efe_inference import InferenceSettings  # absorbed import

from efe_swarm_integration.client import ClassifyRequest, SwarmClient
from efe_swarm_integration.reconciler.decoder import ClassifyResponse

__all__ = ["M06InferenceAdapter"]


@dataclass(slots=True)
class M06InferenceAdapter:
    """Thin wrapper that turns an M06 inference request into an M12
    classification.

    The adapter does NOT try to re-implement M06's pipeline ; it simply
    exposes a ``classify_text(text, settings)`` that M06 callers use
    when they need a swarm verdict.
    """

    swarm: SwarmClient

    async def classify_text(
        self,
        text: str,
        *,
        settings: InferenceSettings,
        tenant_id: str,
        feature_id: str,
        actor_id: str = "spiffe://efe.local/prod/inference",
    ) -> ClassifyResponse:
        req = ClassifyRequest(
            text=text,
            catalog_key=(feature_id, getattr(settings, "task_id", None)),
            tenant_id=tenant_id,
            task_id=getattr(settings, "task_id", None),
            actor_id=actor_id,
            budget_ms=int(getattr(settings, "swarm_budget_ms", 500)),
            max_rounds=int(getattr(settings, "swarm_max_rounds", 8)),
        )
        return await self.swarm.classify(req)


_ = InferenceSettings
