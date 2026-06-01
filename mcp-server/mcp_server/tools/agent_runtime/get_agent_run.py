"""Tool : ``efe_get_agent_run`` — fetch run state + recent steps + pending approvals."""
from __future__ import annotations

from typing import Any

from ..types import Tool, ToolContext
from . import _common


_INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeGetAgentRunInput",
    "description": (
        "Fetch a single agent run by ID. Read-only ; returns run + last "
        "20 steps + pending approvals."
    ),
    "properties": {
        "tenant_id": {"type": "string"},
        "run_id": {"type": "string"},
    },
    "required": ["tenant_id", "run_id"],
    "additionalProperties": False,
}


async def _handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    _common.assert_m25_enabled(ctx)
    _common.assert_tenant_match(ctx, args["tenant_id"])
    flags = await _common.fetch_tenant_flags(ctx, args["tenant_id"])
    _common.assert_dpia_active(flags)
    run = await _common.call_grpc(ctx, "get_agent_run", run_id=args["run_id"])
    return {
        "run": run.get("run"),
        "recent_steps": run.get("recent_steps", []),
        "pending_approvals": run.get("pending_approvals", []),
    }


TOOL: Tool = Tool(
    name="efe_get_agent_run",
    description=_INPUT_SCHEMA["description"],
    input_schema=_INPUT_SCHEMA,
    handler=_handler,
    is_write=False,
    requires_sampling=False,
    profile_tier="agent-native",
    feature_flag="EFE_FEATURE_AGENT_RUNTIME_ENABLED",
)
