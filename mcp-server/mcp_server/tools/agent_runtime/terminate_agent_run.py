"""Tool : ``efe_terminate_agent_run`` — operator force-terminates a run."""
from __future__ import annotations

from typing import Any

from ..types import Tool, ToolContext, ToolError
from . import _common


_INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeTerminateAgentRunInput",
    "description": (
        "Force-terminate an agent run (operator/admin only). The microVM "
        "is SIGKILLed immediately. WRITE — requires `confirm:true` and a "
        "reason ≥ 10 chars."
    ),
    "properties": {
        "tenant_id": {"type": "string"},
        "run_id": {"type": "string"},
        "reason": {"type": "string", "minLength": 10, "maxLength": 2048},
        "confirm": {"type": "boolean"},
    },
    "required": ["tenant_id", "run_id", "reason", "confirm"],
    "additionalProperties": False,
}


async def _handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    _common.assert_m25_enabled(ctx)
    _common.assert_tenant_match(ctx, args["tenant_id"])
    if not args.get("confirm"):
        raise ToolError(
            "confirm=true required to terminate an agent run",
            code="invalid_argument", status_code=400,
        )
    await _common.call_grpc(
        ctx,
        "terminate_agent_run",
        run_id=args["run_id"],
        reason=args["reason"],
    )
    return {"terminated": True, "run_id": args["run_id"]}


TOOL: Tool = Tool(
    name="efe_terminate_agent_run",
    description=_INPUT_SCHEMA["description"],
    input_schema=_INPUT_SCHEMA,
    handler=_handler,
    is_write=True,
    requires_sampling=False,
    profile_tier="agent-native",
    feature_flag="EFE_FEATURE_AGENT_RUNTIME_ENABLED",
)
