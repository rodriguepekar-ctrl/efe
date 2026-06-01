"""Tool : ``efe_approve_agent_step`` — operator approves a pending agent step (Art. 14)."""
from __future__ import annotations

from typing import Any

from ..types import Tool, ToolContext, ToolError
from . import _common


_INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeApproveAgentStepInput",
    "description": (
        "Approve a pending agent step. WRITE — requires "
        "`confirm:true`, a rationale ≥ 10 chars, and the operator/admin "
        "role. Mints a single-use approval token consumed by the next "
        "tool call."
    ),
    "properties": {
        "tenant_id": {"type": "string"},
        "approval_id": {"type": "string"},
        "rationale": {"type": "string", "minLength": 10, "maxLength": 2048},
        "confirm": {"type": "boolean"},
    },
    "required": ["tenant_id", "approval_id", "rationale", "confirm"],
    "additionalProperties": False,
}


async def _handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    _common.assert_m25_enabled(ctx)
    _common.assert_tenant_match(ctx, args["tenant_id"])
    if not args.get("confirm"):
        raise ToolError(
            "confirm=true required to approve an agent step",
            code="invalid_argument", status_code=400,
        )
    flags = await _common.fetch_tenant_flags(ctx, args["tenant_id"])
    _common.assert_dpia_active(flags)

    res = await _common.call_grpc(
        ctx,
        "approve_agent_step",
        approval_id=args["approval_id"],
        reason=args["rationale"],
    )
    return {
        "approval_token": res.get("approval_token"),
        "expires_at": res.get("expires_at"),
    }


TOOL: Tool = Tool(
    name="efe_approve_agent_step",
    description=_INPUT_SCHEMA["description"],
    input_schema=_INPUT_SCHEMA,
    handler=_handler,
    is_write=True,
    requires_sampling=False,
    profile_tier="agent-native",
    feature_flag="EFE_FEATURE_AGENT_RUNTIME_ENABLED",
)
