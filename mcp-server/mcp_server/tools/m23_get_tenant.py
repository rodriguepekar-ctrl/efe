"""efe_get_tenant (D8) — read-only tenant detail."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeGetTenantInput",
    "description": "Read-only tenant detail (plan, quotas, residency, status).",
    "properties": {
        "tenant_id": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$"},
    },
    "required": ["tenant_id"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not hasattr(ctx.backend, "get_tenant"):
        raise ToolError(
            "Backend does not support get_tenant (EFE 1.6 required)",
            code="not_implemented",
        )
    tenant = args["tenant_id"]
    try:
        return await ctx.backend.get_tenant(tenant_id=tenant, auth=ctx.auth)
    except getattr(ctx.backend, "NotFoundError", Exception) as e:
        raise ToolError(
            f"tenant '{tenant}' not found", code="not_found", status_code=404
        ) from e
    except Exception as e:
        raise ToolError(f"get_tenant failed: {e}", code="backend_error") from e


TOOL = Tool(
    name="efe_get_tenant",
    description="Read-only tenant detail.",
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    profile_tier="agent-native",
    feature_flag="D8_ADMIN_ENABLED",
)
