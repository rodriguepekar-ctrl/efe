"""efe_approve_mode3_review (M22 Phase E) — approve, write+confirm."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeApproveMode3ReviewInput",
    "description": (
        "Approve a pending Mode-3 review. Operator can override the "
        "proposed_label with `final_label`. Atomic + audited via "
        "M14-bis chain. WRITE — requires `confirm:true`."
    ),
    "properties": {
        "review_token": {
            "type": "string",
            "format": "uuid",
            "minLength": 36,
            "maxLength": 36,
        },
        "final_label": {
            "type": "string",
            "minLength": 1,
            "maxLength": 100,
            "description": "If null, accepts proposed_label.",
        },
        "review_notes": {
            "type": "string",
            "maxLength": 2048,
        },
        "confirm": {
            "type": "boolean",
            "const": True,
        },
    },
    "required": ["review_token", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "approve_mode3_review requires confirm:true",
            code="confirm_required",
        )
    if not hasattr(ctx.backend, "approve_mode3_review"):
        raise ToolError(
            "Backend does not support Mode-3 review approval (EFE 1.5 required)",
            code="not_implemented",
        )

    token = args["review_token"]
    final_label = args.get("final_label")
    notes = args.get("review_notes")

    try:
        result = await ctx.backend.approve_mode3_review(
            review_token=token,
            final_label=final_label,
            review_notes=notes,
            auth=ctx.auth,
        )
    except Exception as e:
        raise ToolError(
            f"approve_mode3_review failed: {e}",
            code="backend_error",
        ) from e

    return {
        "review_token": token,
        "status": "approved",
        "final_label": result.get("final_label", final_label),
        "reviewed_at": result.get("reviewed_at"),
        "reviewed_by": result.get("reviewed_by"),
    }


TOOL = Tool(
    name="efe_approve_mode3_review",
    description=(
        "Approve a pending Mode-3 review. Operator can override "
        "proposed_label. Atomic + audited (M14-bis chain). WRITE. "
        "Requires `confirm:true` and `EFE_FEATURE_M22_MODE3_ENABLED` "
        "flag."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native",
    feature_flag="M22_MODE3_ENABLED",
)
