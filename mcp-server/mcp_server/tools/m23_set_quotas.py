"""efe_set_quotas (D8) — update tenant quotas. WRITE+confirm."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeSetQuotasInput",
    "description": "Set tenant quotas. WRITE — requires `confirm:true`.",
    "properties": {
        "tenant_id": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$"},
        "max_rps": {"type": "integer", "minimum": 0, "maximum": 100_000},
        "max_tokens_per_day": {"type": "integer", "minimum": 0, "maximum": 1_000_000_000},
        "monthly_cap_usd": {"type": "number", "minimum": 0, "maximum": 1_000_000.0},
        "confirm": {"type": "boolean", "const": True},
    },
    "required": ["tenant_id", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError("set_quotas requires confirm:true", code="confirm_required")
    if not hasattr(ctx.backend, "set_quotas"):
        raise ToolError(
            "Backend does not support set_quotas (EFE 1.6 required)",
            code="not_implemented",
        )
    quotas = {k: args[k] for k in ("max_rps", "max_tokens_per_day", "monthly_cap_usd")
              if k in args}
    if not quotas:
        raise ToolError("supply at least one quota to set", code="invalid_input")
    try:
        return await ctx.backend.set_quotas(
            tenant_id=args["tenant_id"], quotas=quotas, auth=ctx.auth
        )
    except getattr(ctx.backend, "NotFoundError", Exception) as e:
        raise ToolError(
            f"tenant '{args['tenant_id']}' not found",
            code="not_found",
            status_code=404,
        ) from e
    except Exception as e:
        raise ToolError(f"set_quotas failed: {e}", code="backend_error") from e


TOOL = Tool(
    name="efe_set_quotas",
    description="Set tenant quotas. WRITE — requires `confirm:true`.",
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native-deploy",
    feature_flag="D8_ADMIN_ENABLED",
)
