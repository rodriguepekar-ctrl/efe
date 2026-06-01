"""efe_update_tenant (D8) — update tenant fields. WRITE+confirm."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeUpdateTenantInput",
    "description": "Update tenant fields. WRITE — requires `confirm:true`.",
    "properties": {
        "tenant_id": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$"},
        "max_rps": {"type": "integer", "minimum": 0, "maximum": 100_000},
        "max_tokens_per_day": {"type": "integer", "minimum": 0, "maximum": 1_000_000_000},
        "data_residency": {"type": "string", "enum": ["EU_ONLY", "US_ONLY", "any"]},
        "contact_email": {"type": ["string", "null"], "format": "email"},
        "confirm": {"type": "boolean", "const": True},
    },
    "required": ["tenant_id", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError("update_tenant requires confirm:true", code="confirm_required")
    if not hasattr(ctx.backend, "update_tenant"):
        raise ToolError(
            "Backend does not support update_tenant (EFE 1.6 required)",
            code="not_implemented",
        )
    # Extract just the fields the user provided.
    updates = {k: args[k] for k in ("max_rps", "max_tokens_per_day",
                                     "data_residency", "contact_email")
               if k in args}
    if not updates:
        raise ToolError("supply at least one field to update", code="invalid_input")
    try:
        return await ctx.backend.update_tenant(
            tenant_id=args["tenant_id"], updates=updates, auth=ctx.auth
        )
    except getattr(ctx.backend, "NotFoundError", Exception) as e:
        raise ToolError(
            f"tenant '{args['tenant_id']}' not found",
            code="not_found",
            status_code=404,
        ) from e
    except Exception as e:
        raise ToolError(f"update_tenant failed: {e}", code="backend_error") from e


TOOL = Tool(
    name="efe_update_tenant",
    description="Update tenant fields. WRITE — requires `confirm:true`.",
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native-deploy",
    feature_flag="D8_ADMIN_ENABLED",
)
