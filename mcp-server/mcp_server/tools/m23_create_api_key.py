"""efe_create_api_key (D8) — issue a new API key. WRITE+confirm."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeCreateApiKeyInput",
    "description": (
        "Issue a new API key with scope + optional expiration. "
        "Returns plaintext ONCE (caller must store immediately)."
    ),
    "properties": {
        "tenant_id": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$"},
        "scope": {"type": "string", "enum": ["read_only", "classify", "admin"]},
        "expires_at": {"type": ["string", "null"], "format": "date-time"},
        "confirm": {"type": "boolean", "const": True},
    },
    "required": ["tenant_id", "scope", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError("create_api_key requires confirm:true", code="confirm_required")
    if not hasattr(ctx.backend, "create_api_key"):
        raise ToolError(
            "Backend does not support create_api_key (EFE 1.6 required)",
            code="not_implemented",
        )
    try:
        return await ctx.backend.create_api_key(
            tenant_id=args["tenant_id"],
            scope=args["scope"],
            expires_at=args.get("expires_at"),
            auth=ctx.auth,
        )
    except getattr(ctx.backend, "NotFoundError", Exception) as e:
        raise ToolError(
            f"tenant '{args['tenant_id']}' not found",
            code="not_found",
            status_code=404,
        ) from e
    except Exception as e:
        raise ToolError(f"create_api_key failed: {e}", code="backend_error") from e


TOOL = Tool(
    name="efe_create_api_key",
    description="Issue a new API key. Returns plaintext ONCE. WRITE — requires `confirm:true`.",
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native-deploy",
    feature_flag="D8_ADMIN_ENABLED",
)
