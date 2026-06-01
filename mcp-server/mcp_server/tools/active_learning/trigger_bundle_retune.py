"""Tool : ``efe_trigger_bundle_retune`` — on-demand retune."""
from __future__ import annotations

from typing import Any

from ..types import Tool, ToolContext, ToolError
from . import _common


_INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeTriggerBundleRetuneInput",
    "description": (
        "Trigger an on-demand retune of a bundle. Skips the weekly schedule "
        "wait but still respects M24 guardrails (gold_label_threshold, "
        "centroid drift, two-stage eval). force_below_threshold requires "
        "tenant_admin role and bypasses the threshold check."
    ),
    "properties": {
        "tenant_id": {"type": "string"},
        "catalog_id": {"type": "string"},
        "force_below_threshold": {
            "type": "boolean", "default": False,
            "description": "Bypass gold_label_threshold check (tenant_admin only).",
        },
        "confirm": {
            "type": "boolean", "const": True,
            "description": "Must be exactly `true`. Required for every write tool.",
        },
    },
    "required": ["tenant_id", "catalog_id", "confirm"],
    "additionalProperties": False,
}


async def _handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm") is True:
        raise ToolError(
            "confirm=true is required for write tool 'efe_trigger_bundle_retune'",
            code="confirm_required", status_code=400,
        )
    _common.assert_m24_enabled(ctx)
    _common.assert_tenant_match(ctx, args["tenant_id"])
    flags = await _common.fetch_tenant_flags(ctx, args["tenant_id"])
    if not flags.get("m24_active_learning_enabled", False):
        raise ToolError(
            f"M24 not enabled for tenant '{args['tenant_id']}'",
            code="failed_precondition", status_code=412,
        )
    force = bool(args.get("force_below_threshold", False))
    if force:
        # Server-side enforces tenant_admin ; this is just an early hint.
        role = getattr(ctx.auth, "role", "viewer") if ctx.auth else "viewer"
        if role != "tenant_admin":
            raise ToolError(
                "force_below_threshold requires tenant_admin role",
                code="permission_denied", status_code=403,
            )
    resp = await _common.call_grpc(
        ctx, "trigger_retune",
        tenant_id=args["tenant_id"],
        catalog_id=args["catalog_id"],
        force_below_threshold=force,
    )
    return {
        "retune_run_id": resp["retune_run_id"],
        "started_at": resp.get("started_at"),
    }


TOOL: Tool = Tool(
    name="efe_trigger_bundle_retune",
    description=_INPUT_SCHEMA["description"],
    input_schema=_INPUT_SCHEMA,
    handler=_handler,
    is_write=True,
    requires_sampling=False,
    profile_tier="agent-native",
    feature_flag="EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED",
)
