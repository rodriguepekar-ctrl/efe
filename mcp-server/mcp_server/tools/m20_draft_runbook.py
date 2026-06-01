"""efe_draft_runbook (M20 D57) — LLM-helper, sampling-delegated.

Draft an operational runbook from an SLO-breach incident event.
Output is a Markdown body labelled DRAFT — operator review required
before publication.

Intended use cases :
  - SRE on-call receives M19-OBS-* / M19-SIG-* / M19-KMS-* alert
    without a pre-existing runbook → kicks off a draft.
  - Post-incident retrospective : draft an updated runbook from logs.

Default OFF : gated by `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED`.

Pin tests : tests/test_m20_draft_runbook.py.
"""
from __future__ import annotations

import json
from typing import Any

from .m20_prompts import render
from .m20_redaction import redact_for_prompt
from .m20_validation import validate_with_retry
from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeDraftRunbookInput",
    "description": (
        "LLM-helper : produce a markdown runbook DRAFT from an SLO-breach "
        "incident event. Operator review required before publication."
    ),
    "properties": {
        "incident_event": {
            "type": "object",
            "properties": {
                "slo_id": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 50,
                    "pattern": "^[A-Z0-9-_]+$",
                },
                "trigger_metric": {"type": "string", "maxLength": 200},
                "current_value": {"type": "number"},
                "threshold": {"type": "number"},
                "since": {"type": "string", "format": "date-time"},
            },
            "required": ["slo_id"],
            "additionalProperties": False,
        },
        "related_logs": {
            "type": "array",
            "items": {"type": "string", "maxLength": 1024},
            "maxItems": 20,
            "default": [],
        },
        "context_metadata": {
            "type": "object",
            "description": "Free-form context (cluster_id, region, env).",
            "additionalProperties": {"type": "string", "maxLength": 200},
            "default": {},
        },
    },
    "required": ["incident_event"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not ctx.has_sampling():
        raise ToolError(
            "efe_draft_runbook requires MCP sampling.",
            code="sampling_unavailable",
        )

    incident = args["incident_event"]
    related_logs = args.get("related_logs", []) or []
    context_metadata = args.get("context_metadata", {}) or {}

    # Redact PII from logs before prompt inclusion
    redacted_logs = [redact_for_prompt(line) for line in related_logs]
    redacted_metadata = {
        k: redact_for_prompt(str(v)) for k, v in context_metadata.items()
    }

    system_prompt, user_prompt = render(
        "draft_runbook_v1",
        slo_id=incident["slo_id"],
        trigger_metric=str(incident.get("trigger_metric", "(unspecified)")),
        current_value=incident.get("current_value", "?"),
        threshold=incident.get("threshold", "?"),
        since=incident.get("since", "(unknown)"),
        related_logs_json=json.dumps(redacted_logs, ensure_ascii=False),
        context_metadata_json=json.dumps(redacted_metadata, ensure_ascii=False),
    )

    combined_prompt = f"{system_prompt}\n\n{user_prompt}"

    async def _retry_callback(errors: list[str]) -> str:
        retry_prompt = (
            f"{combined_prompt}\n\nValidation errors : {'; '.join(errors)}. "
            f"Produce strictly schema-conformant JSON. The runbook_markdown "
            f"MUST be at least 200 characters and contain ALL required "
            f"sections."
        )
        return await ctx.sampling.sample(
            tenant_id=ctx.tenant_id,
            prompt=retry_prompt,
            max_tokens=3000,
        )

    raw_text = await ctx.sampling.sample(
        tenant_id=ctx.tenant_id,
        prompt=combined_prompt,
        max_tokens=3000,
    )

    output = await validate_with_retry(
        helper="draft_runbook",
        raw_output=raw_text,
        retry_callback=_retry_callback,
        max_retries=2,
    )

    return {
        "runbook_markdown": output["runbook_markdown"],
        "estimated_resolution_time_min": output["estimated_resolution_time_min"],
        "primary_diagnostics": output["primary_diagnostics"],
        "risk_level": output["risk_level"],
        "metadata": {
            "slo_id": incident["slo_id"],
            "n_logs_referenced": len(redacted_logs),
            "prompt_template": "draft_runbook_v1",
            "draft_warning": (
                "This is an LLM-generated DRAFT. Operator review required "
                "before publication / use in production incident response."
            ),
        },
    }


TOOL = Tool(
    name="efe_draft_runbook",
    description=(
        "LLM-helper : draft a markdown runbook from an SLO-breach incident. "
        "Output is a DRAFT requiring operator review. READ-ONLY. Requires "
        "the `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED` flag."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    requires_sampling=True,
    profile_tier="agent-native",
    feature_flag="M20_HELPER_TOOLS_ENABLED",
)
