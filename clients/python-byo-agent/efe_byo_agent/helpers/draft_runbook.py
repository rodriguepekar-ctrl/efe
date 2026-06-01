"""DraftRunbookHelper — direct mode (M20)."""
from __future__ import annotations

import json

from ._base import BaseHelper


class DraftRunbookHelper(BaseHelper):
    name = "efe_draft_runbook"
    helper_id = "draft_runbook"
    template_id = "draft_runbook_v1"

    intelligence_priority = 0.5
    speed_priority = 0.5
    cost_priority = 0.0
    max_tokens = 3000
    temperature = 0.3

    async def run(
        self,
        *,
        incident_event: dict,
        related_logs: list[str] | None = None,
        context_metadata: dict[str, str] | None = None,
    ) -> dict:
        if "slo_id" not in incident_event:
            raise ValueError("incident_event.slo_id is required")
        related_logs = list(related_logs or [])
        context_metadata = dict(context_metadata or {})

        sys_p, usr_p = self._render(
            slo_id=incident_event["slo_id"],
            trigger_metric=str(
                incident_event.get("trigger_metric", "(unspecified)")
            ),
            current_value=incident_event.get("current_value", "?"),
            threshold=incident_event.get("threshold", "?"),
            since=incident_event.get("since", "(unknown)"),
            related_logs_json=json.dumps(related_logs, ensure_ascii=False),
            context_metadata_json=json.dumps(
                context_metadata, ensure_ascii=False,
            ),
        )
        resp = await self._call_llm(sys_p, usr_p)
        validated = await self._validate(resp.text, sys_p, usr_p)

        return {
            "runbook_markdown": validated["runbook_markdown"],
            "estimated_resolution_time_min": validated["estimated_resolution_time_min"],
            "primary_diagnostics": validated["primary_diagnostics"],
            "risk_level": validated["risk_level"],
            "metadata": {
                "slo_id": incident_event["slo_id"],
                "n_logs_referenced": len(related_logs),
                "tokens_in": resp.tokens_in,
                "tokens_out": resp.tokens_out,
                "cost_usd": resp.cost_usd,
                "model_used": resp.model_name,
                "prompt_template": self.template_id,
                "draft_warning": (
                    "This is an LLM-generated DRAFT. Operator review "
                    "required before publication."
                ),
            },
        }


__all__ = ["DraftRunbookHelper"]
