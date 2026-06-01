"""efe_list_api_keys (D8) — read-only API key listing."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeListApiKeysInput",
    "description": "Read-only API key listing. Optional include_revoked.",
    "properties": {
        "tenant_id": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$"},
        "include_revoked": {"type": "boolean", "default": False},
    },
    "required": ["tenant_id"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not hasattr(ctx.backend, "list_api_keys"):
        raise ToolError(
            "Backend does not support list_api_keys (EFE 1.6 required)",
            code="not_implemented",
        )
    try:
        result = await ctx.backend.list_api_keys(
            tenant_id=args["tenant_id"],
            include_revoked=bool(args.get("include_revoked", False)),
            auth=ctx.auth,
        )
    except Exception as e:
        raise ToolError(f"list_api_keys failed: {e}", code="backend_error") from e
    return {"keys": result.get("keys", [])}


TOOL = Tool(
    name="efe_list_api_keys",
    description="Read-only API key listing.",
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    profile_tier="agent-native",
    feature_flag="D8_ADMIN_ENABLED",
)
