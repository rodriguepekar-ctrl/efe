"""Tool : ``efe_create_agent_run`` — instantiate an agent run from a template.

WRITE — requires DPIA + a `confirm:true` flag in the args, and the
caller must hold the `operator` or `admin` role on the tenant.
"""
from __future__ import annotations

from typing import Any

from ..types import Tool, ToolContext, ToolError
from . import _common


_INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeCreateAgentRunInput",
    "description": (
        "Create a new agent run from an AgentRunTemplate. EXPERIMENTAL : "
        "requires DPIA signed for tenant. Tool whitelist may be a SUBSET "
        "of the template's. Cost is hard-capped per run."
    ),
    "properties": {
        "tenant_id": {"type": "string"},
        "template_slug": {"type": "string"},
        "initial_prompt": {
            "type": "object",
            "description": "Free-form JSON ; agent-core interprets as user message + context.",
            "additionalProperties": True,
        },
        "tool_whitelist": {
            "type": "array",
            "items": {"type": "string"},
            "description": "Optional subset override of the template's whitelist.",
        },
        "timeout_seconds": {"type": "integer", "minimum": 60, "maximum": 14400},
        "max_cost_usd": {"type": "number", "minimum": 0.01},
        "confirm": {
            "type": "boolean",
            "description": "Must be true — guard against accidental run creation.",
        },
    },
    "required": ["tenant_id", "template_slug", "initial_prompt", "confirm"],
    "additionalProperties": False,
}


async def _handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    _common.assert_m25_enabled(ctx)
    _common.assert_tenant_match(ctx, args["tenant_id"])
    if not args.get("confirm"):
        raise ToolError(
            "confirm=true required to create an agent run",
            code="invalid_argument", status_code=400,
        )
    flags = await _common.fetch_tenant_flags(ctx, args["tenant_id"])
    _common.assert_dpia_active(flags)

    run = await _common.call_grpc(
        ctx,
        "create_agent_run",
        tenant_id=args["tenant_id"],
        template_slug=args["template_slug"],
        initial_prompt=args["initial_prompt"],
        tool_whitelist=args.get("tool_whitelist"),
        timeout_seconds=args.get("timeout_seconds"),
        max_cost_usd=args.get("max_cost_usd"),
    )
    return {
        "run_id": run.get("id"),
        "status": run.get("status"),
        "tenant_id": run.get("tenant_id"),
        "profile": run.get("profile"),
        "max_cost_cents": run.get("max_cost_cents"),
    }


TOOL: Tool = Tool(
    name="efe_create_agent_run",
    description=_INPUT_SCHEMA["description"],
    input_schema=_INPUT_SCHEMA,
    handler=_handler,
    is_write=True,
    requires_sampling=False,
    profile_tier="agent-native",
    feature_flag="EFE_FEATURE_AGENT_RUNTIME_ENABLED",
)
