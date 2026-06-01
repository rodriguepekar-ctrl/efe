"""efe_delete_tenant (D8) — irreversible tenant deletion. WRITE+confirm."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeDeleteTenantInput",
    "description": (
        "Permanently delete a tenant. Irreversible. Audit chain "
        "preserved for `purge_audit_after_days` days, then deleted "
        "by cron job. WRITE — requires `confirm:true`."
    ),
    "properties": {
        "tenant_id": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$"},
        "purge_audit_after_days": {
            "type": "integer",
            "minimum": 30,           # GDPR floor : 30d minimum retention
            "maximum": 3650,         # 10 years upper bound
            "default": 2555,         # 7 years (M21 audit retention default)
        },
        "confirm": {"type": "boolean", "const": True},
    },
    "required": ["tenant_id", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError("delete_tenant requires confirm:true", code="confirm_required")
    if not hasattr(ctx.backend, "delete_tenant"):
        raise ToolError(
            "Backend does not support delete_tenant (EFE 1.6 required)",
            code="not_implemented",
        )
    try:
        return await ctx.backend.delete_tenant(
            tenant_id=args["tenant_id"],
            purge_audit_after_days=int(args.get("purge_audit_after_days", 2555)),
            auth=ctx.auth,
        )
    except getattr(ctx.backend, "NotFoundError", Exception) as e:
        raise ToolError(
            f"tenant '{args['tenant_id']}' not found",
            code="not_found",
            status_code=404,
        ) from e
    except Exception as e:
        raise ToolError(f"delete_tenant failed: {e}", code="backend_error") from e


TOOL = Tool(
    name="efe_delete_tenant",
    description="Permanently delete a tenant. WRITE — requires `confirm:true`.",
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native-deploy",
    feature_flag="D8_ADMIN_ENABLED",
)
