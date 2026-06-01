"""efe_suspend_tenant (D8) — set max_rps=0 + status=suspended. WRITE+confirm."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeSuspendTenantInput",
    "description": (
        "Suspend a tenant : sets max_rps=0 and status=suspended. "
        "Reversible via `update_tenant` once unblocked. WRITE — "
        "requires `confirm:true`."
    ),
    "properties": {
        "tenant_id": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$"},
        "reason": {"type": "string", "minLength": 10, "maxLength": 1024},
        "confirm": {"type": "boolean", "const": True},
    },
    "required": ["tenant_id", "reason", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError("suspend_tenant requires confirm:true", code="confirm_required")
    if not hasattr(ctx.backend, "suspend_tenant"):
        raise ToolError(
            "Backend does not support suspend_tenant (EFE 1.6 required)",
            code="not_implemented",
        )
    try:
        return await ctx.backend.suspend_tenant(
            tenant_id=args["tenant_id"], reason=args["reason"], auth=ctx.auth
        )
    except getattr(ctx.backend, "NotFoundError", Exception) as e:
        raise ToolError(
            f"tenant '{args['tenant_id']}' not found",
            code="not_found",
            status_code=404,
        ) from e
    except Exception as e:
        raise ToolError(f"suspend_tenant failed: {e}", code="backend_error") from e


TOOL = Tool(
    name="efe_suspend_tenant",
    description="Suspend a tenant. WRITE — requires `confirm:true`.",
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native-deploy",
    feature_flag="D8_ADMIN_ENABLED",
)
