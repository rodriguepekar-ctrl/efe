"""efe_reject_mode3_review (M22 Phase E) — reject, write+confirm."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeRejectMode3ReviewInput",
    "description": (
        "Reject a pending Mode-3 review with a reason. Operator may "
        "trigger manual reclassification out-of-band. WRITE — "
        "requires `confirm:true`. `reason` ≥ 10 chars to prevent "
        "trivial rejections."
    ),
    "properties": {
        "review_token": {
            "type": "string",
            "format": "uuid",
            "minLength": 36,
            "maxLength": 36,
        },
        "reason": {
            "type": "string",
            "minLength": 10,
            "maxLength": 2048,
            "description": "Mandatory rationale, ≥ 10 chars.",
        },
        "confirm": {
            "type": "boolean",
            "const": True,
        },
    },
    "required": ["review_token", "reason", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "reject_mode3_review requires confirm:true",
            code="confirm_required",
        )
    if not hasattr(ctx.backend, "reject_mode3_review"):
        raise ToolError(
            "Backend does not support Mode-3 review rejection (EFE 1.5 required)",
            code="not_implemented",
        )

    token = args["review_token"]
    reason = args["reason"]

    try:
        result = await ctx.backend.reject_mode3_review(
            review_token=token,
            reason=reason,
            auth=ctx.auth,
        )
    except Exception as e:
        raise ToolError(
            f"reject_mode3_review failed: {e}",
            code="backend_error",
        ) from e

    return {
        "review_token": token,
        "status": "rejected",
        "reason": reason,
        "reviewed_at": result.get("reviewed_at"),
        "reviewed_by": result.get("reviewed_by"),
    }


TOOL = Tool(
    name="efe_reject_mode3_review",
    description=(
        "Reject a pending Mode-3 review with a mandatory ≥10-char "
        "reason. Atomic + audited (M14-bis chain). WRITE. Requires "
        "`confirm:true` and `EFE_FEATURE_M22_MODE3_ENABLED` flag."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native",
    feature_flag="M22_MODE3_ENABLED",
)
